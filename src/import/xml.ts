/**
 * 极简 XML 解析器（只够吃 MusicXML）。
 *
 * 为什么不用浏览器自带的 `DOMParser`：解析逻辑要能在 Node 里跑自检（见
 * `tools/import-selftest.ts`），而 Node 没有 DOM。引一个 XML 依赖又太重——MusicXML 是
 * 机器生成的规整 XML，用不着完整的解析器。
 *
 * 支持：元素 / 属性 / 自闭合标签 / 注释 / CDATA / 处理指令 / DOCTYPE / 五种预定义实体 +
 * 数字实体。不支持：命名空间语义、DTD 默认值、外部实体注入（这两样本就不该在乐谱里出现）。
 */

export interface XmlNode {
  name: string;
  attrs: Record<string, string>;
  children: XmlNode[];
  /** 直接子文本（已解实体）。元素之间夹的空白也会进来，取文本时记得 `trim()`。 */
  text: string;
}

/** 解实体。数字实体按码点还原，其余只认 XML 预定义的五种。 */
function decodeEntities(raw: string): string {
  if (!raw.includes("&")) return raw;
  return raw.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (whole, body: string) => {
    if (body.startsWith("#x") || body.startsWith("#X")) {
      const code = Number.parseInt(body.slice(2), 16);
      return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
    }
    if (body.startsWith("#")) {
      const code = Number.parseInt(body.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
    }
    switch (body) {
      case "amp":
        return "&";
      case "lt":
        return "<";
      case "gt":
        return ">";
      case "quot":
        return '"';
      case "apos":
        return "'";
      default:
        return whole;
    }
  });
}

/** 从 `<` 之后找到标签真正的结束 `>`（跳过引号里的内容）。 */
function findTagEnd(input: string, from: number): number {
  let quote = "";
  for (let i = from + 1; i < input.length; i += 1) {
    const c = input[i]!;
    if (quote) {
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === ">") return i;
  }
  return -1;
}

const ATTR_RE = /([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g;

function parseAttrs(raw: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of raw.matchAll(ATTR_RE)) {
    out[m[1]!] = decodeEntities(m[2] ?? m[3] ?? "");
  }
  return out;
}

/**
 * 解析成树。根节点名字固定 `#root`，真正的文档元素是它的第一个子节点。
 *
 * 容错优先：标签不配对时就地停止下探，不抛异常——导入半个坏文件也好过整份丢掉。
 */
export function parseXml(input: string): XmlNode {
  const root: XmlNode = { name: "#root", attrs: {}, children: [], text: "" };
  const stack: XmlNode[] = [root];
  const len = input.length;
  let i = 0;

  const top = (): XmlNode => stack[stack.length - 1]!;

  while (i < len) {
    const lt = input.indexOf("<", i);
    if (lt < 0) {
      top().text += decodeEntities(input.slice(i));
      break;
    }
    if (lt > i) top().text += decodeEntities(input.slice(i, lt));

    if (input.startsWith("<!--", lt)) {
      const end = input.indexOf("-->", lt);
      i = end < 0 ? len : end + 3;
      continue;
    }
    if (input.startsWith("<![CDATA[", lt)) {
      const end = input.indexOf("]]>", lt);
      top().text += input.slice(lt + 9, end < 0 ? len : end);
      i = end < 0 ? len : end + 3;
      continue;
    }
    if (input.startsWith("<?", lt)) {
      const end = input.indexOf("?>", lt);
      i = end < 0 ? len : end + 2;
      continue;
    }
    if (input.startsWith("<!", lt)) {
      // DOCTYPE（可能带内部子集，这里只跳到第一个 `>`，够用）。
      const end = input.indexOf(">", lt);
      i = end < 0 ? len : end + 1;
      continue;
    }

    const gt = findTagEnd(input, lt);
    if (gt < 0) break;
    const raw = input.slice(lt + 1, gt);

    if (raw.startsWith("/")) {
      const name = raw.slice(1).trim();
      for (let s = stack.length - 1; s >= 1; s -= 1) {
        if (stack[s]!.name === name) {
          stack.length = s;
          break;
        }
      }
    } else {
      const selfClosing = raw.endsWith("/");
      const body = selfClosing ? raw.slice(0, -1) : raw;
      const head = /^([^\s/>]+)/.exec(body);
      if (head) {
        const node: XmlNode = {
          name: head[1]!,
          attrs: parseAttrs(body.slice(head[0].length)),
          children: [],
          text: "",
        };
        top().children.push(node);
        if (!selfClosing) stack.push(node);
      }
    }
    i = gt + 1;
  }

  return root;
}

/** 文档元素（跳过 `#root`）。 */
export function documentElement(root: XmlNode): XmlNode | null {
  return root.children.find((c) => c.name !== "#comment") ?? null;
}

/** 第一个同名子元素。 */
export function child(node: XmlNode | null, name: string): XmlNode | null {
  if (!node) return null;
  return node.children.find((c) => c.name === name) ?? null;
}

/** 全部同名子元素。 */
export function children(node: XmlNode | null, name: string): XmlNode[] {
  if (!node) return [];
  return node.children.filter((c) => c.name === name);
}

/** 属性值。 */
export function attr(node: XmlNode | null, name: string): string | null {
  if (!node) return null;
  return node.attrs[name] ?? null;
}

/** 子孙元素（深度优先）。 */
export function descendants(node: XmlNode, name: string): XmlNode[] {
  const out: XmlNode[] = [];
  const walk = (n: XmlNode): void => {
    for (const c of n.children) {
      if (c.name === name) out.push(c);
      walk(c);
    }
  };
  walk(node);
  return out;
}

/** 直接文本（去首尾空白）。 */
export function textOf(node: XmlNode | null): string {
  return (node?.text ?? "").trim();
}
