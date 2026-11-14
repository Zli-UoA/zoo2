# 暗号化チャンネルの echo Bot

暗号化チャンネル（E2EE）で動く Bot のサンプルです。Bot は E2EE の端末として参加し、`!echo <文>` か Bot へのメンションに、同じスレッドへ暗号化した返信を送ります。サーバーには暗号文だけが届きます。仕様は [docs/e2ee.md の「Bot の端末」](../../docs/e2ee.md#bot-の端末)です。

## 準備

1. App の manifest に `e2ee:device` を入れて（[manifest.example.json](manifest.example.json)）、申請とインストールを済ませる。
2. wasm を作る（`nix develop` の中で `crates/zoo-crypto-wasm/build.sh`）。別の場所に置いたときは `ZOO_CRYPTO_WASM_DIR` で指す。
3. 鍵を暗号化する保存鍵を作り、Secret として渡す（リポジトリやログに書かない）。

   ```sh
   export ZOO_E2EE_STORAGE_KEY="$(openssl rand -base64 32)"
   ```

4. 起動する。初回は端末を登録し、クロス署名鍵を作って端末に署名します。鍵は `ZOO_E2EE_STORE`（既定は `e2ee-store.json`）に暗号化して保存します。Zoo は更新のたびに refresh token を替えるので、最新のトークンを `ZOO_CREDENTIALS_FILE`（既定は `credentials.json`。所有者だけが読める）に保存し、次の起動ではこちらを使います。環境変数のトークンは初回の起動にだけ使います。

   ```sh
   ZOO_URL=https://zli.chat ZOO_ACCESS_TOKEN=... ZOO_REFRESH_TOKEN=... npm start
   ```

5. チャンネルのメンバーが、Web の App の詳細から「暗号化チャンネルに追加」を選び、内容を確かめて許可する。

## 注意

- 保存鍵とストアのファイルを両方失うと、Bot は新しい端末とクロス署名鍵を作り直します。メンバーの端末からはマスター鍵が変わったように見え、再検証するまで鍵は届きません。
- 同じ Bot を複数のプロセスで同時に動かさないでください（Bot は 1 つの端末だけを持ちます）。
- 検証済みでない端末から送られたメッセージには応答しません。
- 返信の ID はイベントの ID から決めるので、同じイベントが再送されても返信は 1 つだけです。
- `credentials.json` と `e2ee-store.json` は Secret として扱い、リポジトリに入れないでください。
- やめるときは、チャンネルから Bot を外してください。以降のメッセージは Bot に届きません。

## テスト

```sh
npm test
```

`echo.test.ts` は、メモリー上の Zoo（`sdk/src/testing/fakeZoo.ts`）と、Web と同じ手順で動く人間の端末を使い、Bot が暗号化チャンネルで復号して暗号化した返信を送り、人間の端末がそれを読めることを確かめます。
