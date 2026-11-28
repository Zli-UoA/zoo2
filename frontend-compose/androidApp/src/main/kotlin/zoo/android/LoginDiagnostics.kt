package zoo.android

import io.ktor.client.network.sockets.ConnectTimeoutException
import io.ktor.client.plugins.HttpRequestTimeoutException
import java.io.IOException
import java.net.ConnectException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLException
import zoo.crypto.ReauthException

internal enum class LoginStage(val label: String) {
    Network("ネットワークの確認"),
    Configuration("接続先設定の確認"),
    SavedSession("保存済みセッションの読み込み"),
    Discovery("認証先の設定取得"),
    OpenBrowser("ブラウザーの起動"),
    BrowserCallback("ブラウザーからの復帰"),
    Exchange("認証結果の交換と検証"),
    Restore("保存済み認証の更新"),
    ZooApi("Zoo APIへの接続"),
    Session("アプリのセッション初期化"),
}

internal enum class LoginNetworkIssue(val label: String) {
    Dns("DNS"),
    Certificate("TLS"),
    Timeout("TIMEOUT"),
    Connection("CONNECTION"),
    Io("IO"),
}

internal fun loginNetworkIssue(failure: Throwable): LoginNetworkIssue? {
    for (cause in generateSequence(failure) { it.cause }.take(8)) {
        when (cause) {
            is UnknownHostException -> return LoginNetworkIssue.Dns
            is SSLException -> return LoginNetworkIssue.Certificate
            is ConnectTimeoutException, is HttpRequestTimeoutException, is SocketTimeoutException ->
                return LoginNetworkIssue.Timeout
            is ConnectException -> return LoginNetworkIssue.Connection
        }
    }
    return if (failure is IOException) LoginNetworkIssue.Io else null
}

/** ログイン画面に出す、表示言語のエラー文言。画面に出せる例外（ここで作った文言）でなければ null。 */
internal fun loginErrorText(failure: Throwable, text: LoginText): String? = when (failure) {
    is LoginFailure -> failure.display(text)
    is LoginSettingsInvalid -> failure.display(text)
    is ReauthException -> reauthMessage(failure, text)
    is IllegalArgumentException -> failure.message.orEmpty()
    else -> null
}

private val httpStatus = Regex("""\(HTTP ([1-5][0-9]{2})\)""")

/** Only allowlisted stage names and error codes may be shown or copied. */
internal fun loginDiagnosticLog(stages: List<LoginStage>, failure: Throwable): String {
    val network = loginNetworkIssue(failure)
    val category = when {
        failure is LoginFailure && failure.message == wifiSignInMessage -> "WIFI_SIGN_IN_REQUIRED"
        failure is ReauthException -> failure.reason.name
        network != null -> network.label
        failure is LoginFailure -> "LOGIN_FAILURE"
        failure is IllegalArgumentException -> "INVALID_RESPONSE_OR_SETTINGS"
        else -> "UNEXPECTED"
    }
    val status = when (failure) {
        is LoginFailure, is ReauthException -> httpStatus.find(failure.message.orEmpty())?.groupValues?.get(1)
        else -> null
    }
    val codes = if (status == null) category else "$category · HTTP $status"
    return buildString {
        stages.forEachIndexed { index, stage ->
            appendLine("${index + 1}. ${stage.label} — ${if (index == stages.lastIndex) "失敗" else "完了"}")
        }
        append("エラー分類: $codes")
    }
}
