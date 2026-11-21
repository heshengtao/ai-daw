//! 安装流水线：市场 / npm / 本地目录 → 托管目录 → 热装载。
//!
//! 每一步失败都回滚：临时目录清掉、状态回到安装前、旧版本继续可用（设计 §9）。

use std::path::{Path, PathBuf};

use std::sync::Arc;

use crate::error::{ExtError, ExtResult};
use crate::manifest;
use crate::manager::{package_dir_name, Manager};
use crate::npm::{self, NpmClient, Resolved, VersionRequest};
use crate::state::Source;

/// 安装结果。
#[derive(Debug, Clone)]
pub struct Installed {
    /// 包名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 实际来源。
    pub source: Source,
}

/// 安装器：npm 客户端 + 临时目录。
pub struct Installer {
    npm: NpmClient,
    temp_root: PathBuf,
}

impl Installer {
    /// 建安装器。
    pub fn new(npm: NpmClient, temp_root: impl Into<PathBuf>) -> Self {
        Self {
            npm,
            temp_root: temp_root.into(),
        }
    }

    /// npm 客户端（daemon 拉 registry 文档时也用）。
    pub fn npm(&self) -> &NpmClient {
        &self.npm
    }

    /// 从 npm 装一个版本（`Latest` 会按 `engines.aidaw` 选最高兼容版）。
    pub fn install_npm(
        &self,
        manager: &Arc<Manager>,
        name: &str,
        request: VersionRequest,
        index_url: Option<String>,
    ) -> ExtResult<Installed> {
        let resolved = self.npm.resolve(name, &request, manager.app_version())?;
        self.install_resolved(manager, &resolved, index_url)
    }

    fn install_resolved(
        &self,
        manager: &Arc<Manager>,
        resolved: &Resolved,
        index_url: Option<String>,
    ) -> ExtResult<Installed> {
        std::fs::create_dir_all(&self.temp_root)?;
        let work = self
            .temp_root
            .join(format!("{}-{}", package_dir_name(&resolved.name), resolved.version));
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        std::fs::create_dir_all(&work)?;
        let outcome = self.install_into(manager, resolved, &work, index_url);
        let _ = std::fs::remove_dir_all(&work);
        outcome
    }

    fn install_into(
        &self,
        manager: &Arc<Manager>,
        resolved: &Resolved,
        work: &Path,
        index_url: Option<String>,
    ) -> ExtResult<Installed> {
        // 1. 下载 + 校验
        let tarball = work.join("package.tgz");
        self.npm.download(resolved, &tarball)?;

        // 2. 解包（剥离 package/，挡路径穿越）
        let extracted = work.join("package");
        std::fs::create_dir_all(&extracted)?;
        npm::extract_tarball(&tarball, &extracted)?;

        // 3. 校验包身份与兼容性
        let package = manifest::load_package(&extracted)?;
        if package.name != resolved.name {
            return Err(ExtError::Install(format!(
                "下载到的是「{}」，但请求的是「{}」",
                package.name, resolved.name
            )));
        }
        if package.version != resolved.version {
            return Err(ExtError::Install(format!(
                "下载到的是 {}，但请求的是 {}",
                package.version, resolved.version
            )));
        }
        manifest::check_engine(package.engines_aidaw.as_deref(), manager.app_version())?;

        // 4. 落盘（先搬到版本目录，再写状态）
        let target = manager.package_root(&resolved.name, &resolved.version);
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        move_dir(&extracted, &target)?;

        // 5. 记状态 + 清理旧版本目录
        let source = Source::npm(resolved.registry.clone(), index_url.clone());
        manager.record_installed(&resolved.name, &resolved.version, source.clone(), &target)?;
        prune_old_versions(manager, &resolved.name, &resolved.version);

        // 6. 热装载（失败也保留安装记录与 failed 状态——用户能在管理器里看到原因）
        manager.load(&resolved.name)?;

        Ok(Installed {
            name: resolved.name.clone(),
            version: resolved.version.clone(),
            source,
        })
    }

    /// 从本地目录导入（目录会被**复制**进托管目录：用户随时可能删源目录）。
    pub fn install_local(&self, manager: &Arc<Manager>, source_dir: &Path) -> ExtResult<Installed> {
        let package = manifest::load_package(source_dir)?;
        manifest::check_engine(package.engines_aidaw.as_deref(), manager.app_version())?;

        let target = manager.package_root(&package.name, &package.version);
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Err(err) = copy_dir(&package.root, &target) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(err);
        }

        let source = Source::local(source_dir.display().to_string());
        if let Err(err) = manager.record_installed(&package.name, &package.version, source.clone(), &target) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(err);
        }
        prune_old_versions(manager, &package.name, &package.version);
        manager.load(&package.name)?;

        Ok(Installed {
            name: package.name,
            version: package.version,
            source,
        })
    }

    /// 从本地 `.tgz` 导入。
    pub fn install_tarball(&self, manager: &Arc<Manager>, tarball: &Path) -> ExtResult<Installed> {
        std::fs::create_dir_all(&self.temp_root)?;
        let work = self
            .temp_root
            .join(format!("local-{}", std::process::id()));
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        std::fs::create_dir_all(&work)?;
        let extracted = work.join("package");
        std::fs::create_dir_all(&extracted)?;
        let outcome = (|| {
            npm::extract_tarball(tarball, &extracted)?;
            let package = manifest::load_package(&extracted)?;
            manifest::check_engine(package.engines_aidaw.as_deref(), manager.app_version())?;

            let target = manager.package_root(&package.name, &package.version);
            if target.exists() {
                std::fs::remove_dir_all(&target)?;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            move_dir(&extracted, &target)?;
            let source = Source {
                kind: "tarball".into(),
                path: Some(tarball.display().to_string()),
                ..Default::default()
            };
            manager.record_installed(&package.name, &package.version, source.clone(), &target)?;
            prune_old_versions(manager, &package.name, &package.version);
            manager.load(&package.name)?;
            Ok(Installed {
                name: package.name,
                version: package.version,
                source,
            })
        })();
        let _ = std::fs::remove_dir_all(&work);
        outcome
    }

    /// 卸载。
    pub fn uninstall(&self, manager: &Arc<Manager>, name: &str) -> ExtResult<()> {
        manager.remove_installed(name)
    }
}

fn prune_old_versions(manager: &Manager, name: &str, keep: &str) {
    let root = manager
        .package_root(name, keep)
        .parent()
        .map(Path::to_path_buf);
    let Some(root) = root else { return };
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() == keep {
            continue;
        }
        if entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// 目录搬移（同一分区 rename；跨分区退化成复制 + 删除）。
fn move_dir(from: &Path, to: &Path) -> ExtResult<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_dir(from, to)?;
            std::fs::remove_dir_all(from)?;
            Ok(())
        }
    }
}

/// 递归复制目录（跳过 `.git` 与悬空软链）。
fn copy_dir(from: &Path, to: &Path) -> ExtResult<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let source = entry.path();
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target)?;
        }
        // 软链与设备文件：跳过（也是攻击面）。
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::NoopBridge;
    use crate::manager::ManagerConfig;
    use std::io::Write;

    fn manager(dir: &Path) -> Arc<Manager> {
        let config = ManagerConfig::new(dir.join("data"), dir.join("config"), "1.9.0");
        let manager = Manager::open(config).unwrap();
        manager.set_bridge(Arc::new(NoopBridge));
        manager
    }

    /// 造一个合法的扩展目录。
    fn fixture(dir: &Path, name: &str, version: &str) {
        std::fs::write(
            dir.join("package.json"),
            format!(
                r#"{{"name":"{name}","version":"{version}","aidaw":{{"manifest":"./aidaw.json"}},"engines":{{"aidaw":">=1.9.0"}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("aidaw.json"),
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs","capabilities":["tools"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("host.cjs"),
            r#"module.exports = { activate: function (aidaw) { aidaw.registerTool({ name: "hello_world", description: "d", parameters: {} }, function () { return "hi"; }); } };"#,
        )
        .unwrap();
    }

    #[test]
    fn installs_from_a_local_directory_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path());
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        fixture(&source, "@aidaw/hello", "0.1.0");

        let installer = Installer::new(NpmClient::new(vec![]), dir.path().join("tmp"));
        let installed = installer.install_local(&manager, &source).unwrap();
        assert_eq!(installed.name, "@aidaw/hello");
        assert_eq!(installed.version, "0.1.0");
        assert_eq!(manager.call_tool("hello_world", "{}").unwrap(), "\"hi\"");
        assert!(manager.package_root("@aidaw/hello", "0.1.0").join("package.json").is_file());
    }

    #[test]
    fn local_import_rejects_incompatible_engine() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path());
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("package.json"),
            r#"{"name":"@aidaw/old","version":"1.0.0","aidaw":{"manifest":"./aidaw.json"},"engines":{"aidaw":">=9.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(source.join("aidaw.json"), r#"{"apiVersion":1,"id":"old","host":"./h.cjs"}"#).unwrap();
        std::fs::write(source.join("h.cjs"), "module.exports={activate:function(){}};").unwrap();

        let installer = Installer::new(NpmClient::new(vec![]), dir.path().join("tmp"));
        let err = installer.install_local(&manager, &source).unwrap_err();
        assert!(err.to_string().contains("9.0.0"), "{err}");
        assert!(manager.state().installed.is_empty());
    }

    #[test]
    fn uninstall_removes_files_and_registrations() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path());
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        fixture(&source, "@aidaw/hello", "0.1.0");
        let installer = Installer::new(NpmClient::new(vec![]), dir.path().join("tmp"));
        installer.install_local(&manager, &source).unwrap();
        installer.uninstall(&manager, "@aidaw/hello").unwrap();
        assert!(manager.tools().is_empty());
        assert!(manager.state().installed.is_empty());
        assert!(!manager.package_root("@aidaw/hello", "0.1.0").exists());
    }

    #[test]
    fn installing_a_newer_version_prunes_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path());
        let installer = Installer::new(NpmClient::new(vec![]), dir.path().join("tmp"));
        for version in ["0.1.0", "0.2.0"] {
            let source = dir.path().join(format!("source-{version}"));
            std::fs::create_dir_all(&source).unwrap();
            fixture(&source, "@aidaw/hello", version);
            installer.install_local(&manager, &source).unwrap();
        }
        assert!(manager.package_root("@aidaw/hello", "0.2.0").exists());
        assert!(!manager.package_root("@aidaw/hello", "0.1.0").exists());
        assert_eq!(manager.state().installed["@aidaw/hello"].version, "0.2.0");
    }

    #[test]
    fn tarball_import_works() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path());
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        fixture(&source, "@aidaw/hello", "0.1.0");

        // 打成 npm 形状的 tgz。
        let tarball = dir.path().join("hello.tgz");
        {
            let file = std::fs::File::create(&tarball).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            for name in ["package.json", "aidaw.json", "host.cjs"] {
                let bytes = std::fs::read(source.join(name)).unwrap();
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, format!("package/{name}"), &bytes[..])
                    .unwrap();
            }
            builder.finish().unwrap();
            let mut file = std::fs::File::open(&tarball).unwrap();
            let mut sink = Vec::new();
            std::io::copy(&mut file, &mut sink).unwrap();
            let mut out = std::fs::File::create(&tarball).unwrap();
            out.write_all(&sink).unwrap();
        }

        let installer = Installer::new(NpmClient::new(vec![]), dir.path().join("tmp"));
        let installed = installer.install_tarball(&manager, &tarball).unwrap();
        assert_eq!(installed.version, "0.1.0");
        assert_eq!(manager.state().installed["@aidaw/hello"].source.kind, "tarball");
    }

    #[test]
    fn copy_dir_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a");
        std::fs::create_dir_all(source.join(".git")).unwrap();
        std::fs::write(source.join(".git").join("x"), "x").unwrap();
        std::fs::write(source.join("f.txt"), "y").unwrap();
        let target = dir.path().join("b");
        copy_dir(&source, &target).unwrap();
        assert!(target.join("f.txt").is_file());
        assert!(!target.join(".git").exists());
    }
}
