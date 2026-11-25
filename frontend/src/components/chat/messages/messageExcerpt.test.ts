import { describe, expect, it } from "vite-plus/test";

import { EXCERPT_MAX_LENGTH, messageExcerpt } from "./messageExcerpt";

describe("messageExcerpt", () => {
  it("素のテキストはそのまま返す", () => {
    expect(messageExcerpt("テスト")).toBe("テスト");
  });

  it("空・空白だけの本文は空文字にする", () => {
    expect(messageExcerpt("")).toBe("");
    expect(messageExcerpt("  \n\t ")).toBe("");
  });

  it("改行と連続する空白を 1 つの空白にまとめる", () => {
    expect(messageExcerpt("一行目\n\n二行目   三行目\t四")).toBe("一行目 二行目 三行目 四");
  });

  it.each([
    ["太字", "**重要**なお知らせ", "重要なお知らせ"],
    ["斜体・取り消し線", "<i>斜</i>と~~消~~", "斜と消"],
    ["小さい文字", "<small>注</small>釈", "注釈"],
    ["MFM の関数", "$[tada 祝！] 完了", "祝！ 完了"],
    ["入れ子の装飾", "**$[spin <i>回る</i>]**", "回る"],
    ["中央寄せ", "<center>中央</center>\nです", "中央 です"],
    ["plain", "<plain>**そのまま**</plain>", "**そのまま**"],
  ])("%s の記法を外す", (_name, content, expected) => {
    expect(messageExcerpt(content)).toBe(expected);
  });

  it("リンクは、文字だけを残す（リンクの中にリンクを作らない）", () => {
    expect(messageExcerpt("[ドキュメント](https://example.com/docs)を確認")).toBe(
      "ドキュメントを確認",
    );
  });

  it("URL はそのまま残す", () => {
    expect(messageExcerpt("見て https://example.com/a?b=1 です")).toBe(
      "見て https://example.com/a?b=1 です",
    );
  });

  it("メンション・ハッシュタグ・絵文字は読める形で残す", () => {
    expect(messageExcerpt("@alice #general :tada: 🎉")).toBe("@alice #general :tada: 🎉");
    expect(messageExcerpt("@bob@example.com へ")).toBe("@bob@example.com へ");
  });

  it("コードは中身だけを残す", () => {
    expect(messageExcerpt("`npm test` を実行")).toBe("npm test を実行");
    expect(messageExcerpt("前\n```js\nconst a = 1;\n```\n後")).toBe("前 const a = 1; 後");
  });

  it("引用は前後の文と単語をつなげない", () => {
    expect(messageExcerpt("前\n> 引用\n後")).toBe("前 引用 後");
  });

  it("数式は式の文字を残す", () => {
    expect(messageExcerpt("\\(x^2\\) です")).toBe("x^2 です");
  });

  it("検索の記法は、検索語だけを残す", () => {
    expect(messageExcerpt("今日の天気 検索")).toBe("今日の天気");
  });

  it("長い本文は、上限の文字数で切って省略記号を付ける", () => {
    const long = "あ".repeat(EXCERPT_MAX_LENGTH + 50);
    const excerpt = messageExcerpt(long);
    expect(Array.from(excerpt)).toHaveLength(EXCERPT_MAX_LENGTH + 1);
    expect(excerpt.endsWith("…")).toBe(true);
  });

  it("上限ちょうどの本文は切らない", () => {
    const exact = "あ".repeat(EXCERPT_MAX_LENGTH);
    expect(messageExcerpt(exact)).toBe(exact);
  });

  it("サロゲートペアの途中で切らない", () => {
    const excerpt = messageExcerpt("😀".repeat(10), 3);
    expect(excerpt).toBe("😀😀😀…");
  });

  it("切ったあとの末尾の空白は落とす", () => {
    expect(messageExcerpt("abc def", 4)).toBe("abc…");
  });

  it("HTML はタグとして解釈せず、文字のまま残す", () => {
    expect(messageExcerpt("<script>alert(1)</script>")).toBe("<script>alert(1)</script>");
  });
});
