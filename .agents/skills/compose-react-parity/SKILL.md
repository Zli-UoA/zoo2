---
name: compose-react-parity
description: Compose Web実験中止後の保守境界と、継続するAndroidへの影響を確認する。
---

# Compose Web 実験中止後の方針

Compose for Web は動作が重く実用に耐えないため実験を中止した。Web は React で開発する。
React の変更や main の取り込みで Compose Web の追従・描画比較・Wasm ビルド・Web E2E・Storybook の Kotlin 登録を要求しない。

Compose for Android は継続する。共通 Kotlin、API、認証、Electric 同期、翻訳・生成資源を変更するときは Android の呼び出し元と依存を確認し、必要な修正と検証を同じ PR に含める。
Android のビルド・単体テスト・lint は `docs/android.md` の手順と CI Android を使う。Web 専用の検証を Android の検証の代わりにしない。

Web 専用コード・テスト・計測結果は過去の実験資料として保持する。削除時は Android が使う commonMain、androidMain、androidApp、Electric4k、翻訳・フォント・アイコン生成を巻き込まない。
実験終了の判断と保持範囲は `docs/compose-multiplatform.md`、過去の対応表は `docs/compose-parity.md` を参照する。
