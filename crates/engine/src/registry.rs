use std::collections::HashMap;

use daw_party_plugin_api::Plugin;

type Factory = Box<dyn Fn() -> Box<dyn Plugin> + Send + Sync>;

/// 插件注册表：按 id 创建插件实例。
#[derive(Default)]
pub struct PluginRegistry {
    factories: HashMap<String, Factory>,
}

impl PluginRegistry {
    /// 创建一个空注册表。
    pub fn new() -> Self {
        PluginRegistry { factories: HashMap::new() }
    }

    /// 内置插件：目前只有合成器。
    pub fn with_builtins() -> Self {
        let mut r = PluginRegistry::new();
        r.register("builtin.synth", || Box::new(daw_party_synth::SynthPlugin::new()));
        r
    }

    /// 注册一个插件工厂，同 id 覆盖旧值。
    pub fn register(&mut self, id: &str, factory: impl Fn() -> Box<dyn Plugin> + Send + Sync + 'static) {
        self.factories.insert(id.to_string(), Box::new(factory));
    }

    /// 按 id 创建插件实例；未知 id 返回 `None`。
    pub fn create(&self, id: &str) -> Option<Box<dyn Plugin>> {
        self.factories.get(id).map(|f| f())
    }

    /// 已注册的全部插件 id（升序排列）。
    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.factories.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    // `Plugin` 已由上方模块级 `use daw_party_plugin_api::Plugin;` 经 `super::*` 引入。
    use super::*;

    #[test]
    fn builtins_include_synth() {
        let r = PluginRegistry::with_builtins();
        assert!(r.ids().contains(&"builtin.synth".to_string()));
    }

    #[test]
    fn create_returns_configured_instance() {
        let r = PluginRegistry::with_builtins();
        let p = r.create("builtin.synth").expect("synth exists");
        assert_eq!(p.descriptor().id, "builtin.synth");
        assert_eq!(p.get_param(&"gain".into()), Some(0.8));
    }

    #[test]
    fn create_unknown_returns_none() {
        let r = PluginRegistry::with_builtins();
        assert!(r.create("nope").is_none());
    }
}
