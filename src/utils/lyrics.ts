/**
 * 歌词切词（与后端 `singing::tokenize` 同规则）。
 *
 * - 汉字：一字一个记号，紧跟的 `1..5` 是强制声调（`小4` 是一个记号）；
 * - `-`：连音记号（一个 `-` 吃掉一个音）；
 * - 拉丁词：整体当一个记号（英文按词唱，不做逐字母切分）；
 * - 标点 / 空白：分隔符，不占记号。
 *
 * 钢琴卷帘的歌词模式与 NoteEditorDialog 的「按顺序填入」共用这一份，避免两边规则漂移。
 */
export function splitLyrics(text: string): string[] {
  const chars = Array.from(text);
  const out: string[] = [];
  for (let i = 0; i < chars.length; i += 1) {
    const c = chars[i];
    if (/\s/.test(c) || /[，。！？、；：,.!?;:"'“”‘’（）()【】[\]]/.test(c)) continue;
    if (c === "-" || c === "—") {
      out.push("-");
      continue;
    }
    if (/[\u4e00-\u9fff]/.test(c)) {
      const digit = chars[i + 1];
      if (digit && /[1-5]/.test(digit)) {
        out.push(c + digit);
        i += 1;
      } else {
        out.push(c);
      }
      continue;
    }
    if (/[A-Za-z0-9]/.test(c)) {
      let word = c;
      while (i + 1 < chars.length && /[A-Za-z0-9']/.test(chars[i + 1])) {
        word += chars[i + 1];
        i += 1;
      }
      out.push(word);
    }
  }
  return out;
}

/**
 * 一个音符最多能唱一个记号：取第一个。
 *
 * 中文就是「一个汉字（可带声调）」；英文是一个词。多出来的字由调用方自动填到后面的
 * 音符上（见 `EditView` 的歌词模式），而不是硬塞进这一个音。
 */
export function firstLyricToken(text: string): string {
  return splitLyrics(text)[0] ?? "";
}
