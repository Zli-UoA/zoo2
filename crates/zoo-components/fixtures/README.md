# 部品の fixture

Web・Android・Tauri の描画と、backend の検証が同じ文書を扱えることを確かめるための JSON です。

- `valid/<context>-<name>.json`：その context（`message`・`modal`・`home`・`webhook`）で検証に通る文書。`fallback` があれば、`fallback_text` の結果と一致すること
- クライアントのテストは、この JSON を読んで、知らない部品で落ちないこと・本文を隠すかどうか（`flags`）を確かめる
