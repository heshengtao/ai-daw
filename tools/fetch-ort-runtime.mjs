#!/usr/bin/env node
/**
 * 下载**当前平台**的 ONNX Runtime 运行时（随安装包分发），解出需要的文件到
 * `src-tauri/resources/ort/`。
 *
 * 为什么要这一步：`ort` 现在用 `load-dynamic`（为了能按需下载 CUDA），运行时不再静态链接进
 * exe。CPU / DirectML / CoreML 这三份按用户要求**随包**，于是构建前把它们放进资源目录，
 * 由 Tauri 的 `bundle.resources` 打进安装包（见 `tauri.conf.json` 的 `resources/ort`）。
 *
 * 只取需要的文件：
 * - Windows：DirectML 构建（含 CPU EP，一份顶两个后端）+ `DirectML.dll`；
 * - macOS arm64：含 CoreML 的构建；
 * - Linux x64：CPU 构建。
 *
 * 用法：`npm run fetch:ort`（已有文件就跳过，`--force` 重下）。
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, copyFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const OUT = join(ROOT, "src-tauri", "resources", "ort");
const FORCE = process.argv.includes("--force");

const ORT = "1.30.0";
const NUGET = "https://api.nuget.org/v3-flatcontainer";
const GH = `https://github.com/microsoft/onnxruntime/releases/download/v${ORT}`;

/** 当前平台要准备什么：目标文件名 → 来源。 */
function plan() {
  if (process.platform === "win32" && process.arch === "x64") {
    return {
      files: ["onnxruntime.dll", "DirectML.dll"],
      // DirectML 构建：同时提供 CPU 与 DirectML 两个 EP（1.24.4 是这个包的最后版本）。
      steps: [
        {
          url: `${NUGET}/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg`,
          kind: "zip",
          pick: { "runtimes/win-x64/native/onnxruntime.dll": "onnxruntime.dll" },
        },
        {
          url: `${NUGET}/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg`,
          kind: "zip",
          pick: { "bin/x64-win/DirectML.dll": "DirectML.dll" },
        },
      ],
    };
  }
  // macOS：ONNX Runtime 是**静态链接**进 app 的（那里没有 CUDA，不需要运行期换 dylib），
  // 所以没有随包文件要准备。
  if (process.platform === "linux" && process.arch === "x64") {
    return {
      files: ["libonnxruntime.so"],
      steps: [
        {
          url: `${GH}/onnxruntime-linux-x64-${ORT}.tgz`,
          kind: "tgz",
          pick: { [`onnxruntime-linux-x64-${ORT}/lib/libonnxruntime.so.${ORT}`]: "libonnxruntime.so" },
        },
      ],
    };
  }
  return null;
}

async function download(url, dest) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`下载失败 ${res.status}：${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  const { writeFileSync } = await import("node:fs");
  writeFileSync(dest, buf);
  return buf.length;
}

function extract(archive, kind, into) {
  mkdirSync(into, { recursive: true });
  if (kind === "zip") {
    if (process.platform === "win32") {
      execFileSync(
        "powershell",
        ["-NoProfile", "-Command", `Expand-Archive -Force -LiteralPath '${archive}' -DestinationPath '${into}'`],
        { stdio: "inherit" },
      );
    } else {
      execFileSync("unzip", ["-o", "-q", archive, "-d", into], { stdio: "inherit" });
    }
  } else {
    execFileSync("tar", ["-xzf", archive, "-C", into], { stdio: "inherit" });
  }
}

function find(dir, relative) {
  // 归档里的路径分隔符统一成 `/`。
  const target = relative.split("/").join(process.platform === "win32" ? "\\" : "/");
  const full = join(dir, target);
  if (existsSync(full)) return full;
  // Windows 的 nupkg 解出来是 `runtimes\win-x64\native\...`，join 已经处理；再兜一层大小写。
  const stack = [dir];
  while (stack.length) {
    const current = stack.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) stack.push(path);
      else if (path.toLowerCase().endsWith(target.toLowerCase())) return path;
    }
  }
  return null;
}

async function main() {
  const p = plan();
  // 目录必须先建出来，**哪怕当前平台没有要下载的文件**：`tauri.conf.json` 的
  // `bundle.resources` 里写死了 `resources/ort`，Tauri 的构建脚本遇到不存在的资源路径会
  // 直接报 `resource path ... doesn't exist`。macOS arm64 走的就是这条路（ORT 静态链接），
  // 不建目录的话 `pnpm tauri dev` 连编译都过不去。
  mkdirSync(OUT, { recursive: true });
  if (!p) {
    console.log(`[ort] 跳过：${process.platform}/${process.arch} 没有随包运行时（目录已就绪）`);
    return;
  }
  if (!FORCE && p.files.every((name) => existsSync(join(OUT, name)))) {
    const mb = p.files.reduce((sum, n) => sum + statSync(join(OUT, n)).size, 0) / 1048576;
    console.log(`[ort] 已就绪（${mb.toFixed(1)} MB）：${p.files.join(", ")}`);
    return;
  }

  const work = join(tmpdir(), `ai-daw-ort-${Date.now()}`);
  mkdirSync(work, { recursive: true });
  try {
    for (const [index, step] of p.steps.entries()) {
      // 临时名必须带正确扩展名（PowerShell 的 Expand-Archive 只认 .zip）。
      const name = `step-${index}.${step.kind === "zip" ? "zip" : "tgz"}`;
      const archive = join(work, name);
      const bytes = await download(step.url, archive);
      console.log(`[ort] 下载 ${name}（${(bytes / 1048576).toFixed(1)} MB）`);
      const unpacked = join(work, `${name}.dir`);
      extract(archive, step.kind, unpacked);
      for (const [inner, target] of Object.entries(step.pick)) {
        const found = find(unpacked, inner);
        if (!found) throw new Error(`归档里没有 ${inner}`);
        copyFileSync(found, join(OUT, target));
        console.log(`[ort]   ${target} ← ${inner}`);
      }
    }
    const mb = p.files.reduce((sum, n) => sum + statSync(join(OUT, n)).size, 0) / 1048576;
    console.log(`[ort] 完成：src-tauri/resources/ort（${mb.toFixed(1)} MB）`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(`[ort] 失败：${error.message}`);
  process.exit(1);
});
