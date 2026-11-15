// E2EE の再認証のリダイレクト先（docs/e2ee.md の「初回設定と解錠の流れ」の 7。#360）。
//
// 元のページ（同じオリジン）に認可コードと state だけを返す。PKCE の code verifier は元のページの
// メモリーにだけあり、このページは知らない。送り先は自分のオリジンに固定し、元のページはオリジンと
// state を確かめる。AuthZ を経由したあとに window.opener が切れる環境（Cross-Origin-Opener-Policy）
// のために、同じオリジンだけに届く BroadcastChannel でも送る。
(function () {
  var params = new URLSearchParams(window.location.search);
  var message = {
    type: "zoo-reauth",
    code: params.get("code"),
    state: params.get("state"),
    error: params.get("error"),
  };
  // 認可コードを履歴に残さない。
  window.history.replaceState(null, "", window.location.pathname);
  try {
    if (window.opener) window.opener.postMessage(message, window.location.origin);
  } catch {
    // opener が別のオリジンに移った場合など。BroadcastChannel で届く。
  }
  try {
    var channel = new BroadcastChannel("zoo-reauth");
    channel.postMessage(message);
    channel.close();
  } catch {
    // BroadcastChannel がない環境では postMessage だけを使う。
  }
  window.close();
})();
