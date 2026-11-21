# 插件商店索引（`index.json`）

扩展的「发现」靠一份**静态 JSON**：插件商店仓库里放 `index.json`（连同可选的图标目录），
应用按设置页里配置的地址去读它。仓库本身不需要服务端，GitHub 的 raw 地址就够。

默认地址是代码里的常量 `ai_daw_extensions::state::DEFAULT_INDEX_URL`
（`crates/extensions/src/state.rs`）：

```
https://heshengtao.github.io/ai-daw-plugins/index.json
```

仓库：<https://github.com/heshengtao/ai-daw-plugins>（Pages 从 `main` 分支根目录发布）。
raw 地址 `https://raw.githubusercontent.com/heshengtao/ai-daw-plugins/main/index.json`
同样能读，但在部分网络下会被 reset——这也是默认用 Pages 的原因。用户可以在「扩展 → 设置」
里覆盖或追加源。

## 仓库长什么样

```
ai-daw-plugins/
├── plugins.txt      # 维护用：一行一个 npm 包名
├── index.json       # 生成产物，应用读它
├── icons/           # 可选：URL 图标（大于 32 KiB 的内联上限时就放这里）
└── .github/workflows/refresh.yml   # 可选：定时重新生成
```

`plugins.txt`：

```
# 每行一个 npm 包名
@aidaw/hello-extension
some-other-plugin
```

生成：

```sh
node tools/build-extension-index.mjs \
  --input plugins.txt --output index.json \
  --title "AI DAW Extensions" --description "社区扩展"
```

生成器只读 npm registry 元数据（不下载、不执行包代码），输出的每个版本都带
`engines.aidaw`；没有声明这个字段的版本会被跳过（应用侧的兼容性判定是硬要求，不做猜测）。

## 格式（format 1）

字段与 DeepSeek Harness 的插件市场格式一致，只把 `engines.dsh` 换成 `engines.aidaw`：

```jsonc
{
  "format": 1,
  "title": { "zh": "AI DAW 扩展", "en": "AI DAW Extensions" },
  "metadata": {
    "categories": { "tools": { "title": { "zh": "工具", "en": "Tools" } } },
    "labels": { "official": { "title": { "zh": "官方", "en": "Official" } } }
  },
  "plugins": [
    {
      "name": "@aidaw/hello-extension",
      "title": { "zh": "示例扩展", "en": "Hello Extension" },
      "description": { "zh": "…", "en": "…" },
      "icon": "icons/hello.svg",              // 相对索引地址；或 data:image/...;base64,...（≤32 KiB）
      "author": "AI DAW",
      "metadata": { "category": "tools", "labels": ["official"], "keywords": ["示例"] },
      "dist-tags": { "latest": "0.1.0" },
      "versions": [
        { "version": "0.1.0", "publishTimestamp": 1795000000,
          "source": { "type": "npm" }, "engines": { "aidaw": ">=1.9.0 <2.0.0" } }
      ]
    }
  ]
}
```

读取器的纪律（和 DSH 一致）：

- **整份拒绝**，不静默丢条目——格式错是生成器的 bug，报错带 JSON 路径；
- 未知字段忽略（向前兼容）；
- `engines.aidaw` 必填且必须是合法 range（npm 写法：空格分隔、`||`、`1.2.3 - 2.0.0` 都支持）；
- `dist-tags` 只能指向已列出的精确版本；
- 图标：`data:` 内联至多 32 KiB，URL 必须是 https 或与索引同源；
- `source.type` 目前**只支持 `npm`**（`git` 保留在格式里，但本版读取会明确拒绝）。

## 安全边界

索引是**无签名**的：标题、作者、标签都是市场自述，不代表验证。应用会显示索引地址与
安装来源；安装本身是「执行第三方代码」，必须用户确认（MCP 侧要求 `confirm=true`）。

## 发布流程

1. 扩展作者 `npm publish`（包里有 `aidaw.manifest` 与 `engines.aidaw`）。
2. 维护者把包名加进 `plugins.txt`，跑生成器，提交 `index.json`。
3. 应用下一次刷新（缓存 TTL 10 分钟，或点「刷新」）就能看到新版本。
