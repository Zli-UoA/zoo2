package zoo.android.update

import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.annotation.Keep
import java.io.IOException
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit

/**
 * Runs as shell/root inside Shizuku; the app shares a read-only FD, never a shell-readable path.
 */
@Keep
class UpdateInstallerService : IUpdateInstaller.Stub() {
    init {
        // Self-update kills the client before its finally block can unbind this daemon.
        // Bound the daemon lifetime even when that client never returns.
        Thread {
                Thread.sleep(180_000)
                kotlin.system.exitProcess(0)
            }
            .apply {
                isDaemon = true
                start()
            }
    }

    override fun destroy() {
        kotlin.system.exitProcess(0)
    }

    @Synchronized
    override fun install(apk: ParcelFileDescriptor, size: Long, userId: Int): String {
        require(size in 1..MAX_APK_BYTES && userId >= 0)
        Log.i(
            "ZooUpdaterService",
            "Install started: uid=${android.os.Process.myUid()}, user=$userId, bytes=$size",
        )
        val process =
            ProcessBuilder(
                    "/system/bin/pm",
                    "install",
                    "-r",
                    "--user",
                    userId.toString(),
                    "--pkg",
                    "works.zli.zoo",
                    "-S",
                    size.toString(),
                    // End option parsing before the stdin path; some Android
                    // versions otherwise reject '-' as an unknown option.
                    "--",
                    "-",
                )
                .redirectErrorStream(true)
                .start()
        // Drain output concurrently so neither pipe can deadlock the installer.
        val output = java.io.ByteArrayOutputStream()
        val reader =
            Thread {
                    process.inputStream.use { input ->
                        val buffer = ByteArray(4096)
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            if (output.size() < 16_384)
                                output.write(buffer, 0, minOf(count, 16_384 - output.size()))
                        }
                    }
                }
                .apply {
                    isDaemon = true
                    start()
                }
        val watchdog =
            Thread {
                    try {
                        if (!process.waitFor(120, TimeUnit.SECONDS)) process.destroyForcibly()
                    } catch (_: InterruptedException) {
                        process.destroy()
                    }
                }
                .apply {
                    isDaemon = true
                    start()
                }
        return try {
            ParcelFileDescriptor.AutoCloseInputStream(apk).use { input ->
                process.outputStream.use { input.copyTo(it) }
            }
            process.waitFor()
            reader.join(5000)
            val result = output.toString(Charsets.UTF_8.name()).trim()
            Log.i(
                "ZooUpdaterService",
                "Install finished: exit=${process.exitValue()}, output=${result.take(1000)}",
            )
            completeInstall(process.exitValue(), result) { restartApp(userId) }
        } catch (failure: IOException) {
            // pm may reject the install before consuming stdin. Preserve its
            // diagnostic instead of sending an IOException through Binder.
            val exited = process.waitFor(5, TimeUnit.SECONDS)
            if (!exited) process.destroyForcibly()
            reader.join(5000)
            val result = output.toString(Charsets.UTF_8.name()).trim().take(1000)
            val exit = if (exited) process.exitValue().toString() else "timeout"
            Log.e(
                "ZooUpdaterService",
                "Install transfer failed: exit=$exit, output=$result",
                failure,
            )
            "$INSTALL_FAILED_PREFIX${result.ifBlank { failure.toString() }} (exit=$exit)"
        } catch (failure: Exception) {
            Log.e("ZooUpdaterService", "Install failed", failure)
            throw failure
        } finally {
            process.destroy()
            watchdog.interrupt()
        }
    }
    // The Zoo process is replaced by Package Manager. Relaunch from this shell daemon instead.
    private fun restartApp(userId: Int) {
        try {
            val launch = ProcessBuilder(restartCommand(userId)).redirectErrorStream(true).start()
            val rejected = FutureTask {
                launch.inputStream.bufferedReader().use { reader ->
                    var failure = false
                    reader.lineSequence().forEach { raw ->
                        val line = raw.trim()
                        if (line.startsWith("Error", ignoreCase = true) ||
                            line.contains("SecurityException", ignoreCase = true) ||
                            line.startsWith("Exception", ignoreCase = true) ||
                            line.startsWith("Status: timeout", ignoreCase = true)
                        ) failure = true
                    }
                    failure
                }
            }
            Thread(rejected, "Zoo restart output").apply { isDaemon = true; start() }
            try {
                if (!launch.waitFor(10, TimeUnit.SECONDS)) {
                    Log.w("ZooUpdaterService", "Zoo restart timed out")
                } else if (rejected.get(2, TimeUnit.SECONDS)) {
                    Log.w("ZooUpdaterService", "Android rejected the restart after installation")
                } else if (launch.exitValue() != 0) {
                    Log.w("ZooUpdaterService", "Zoo restart failed with exit code ${launch.exitValue()}")
                }
            } finally {
                launch.destroyForcibly()
                rejected.cancel(true)
            }
        } catch (failure: Exception) {
            Log.w("ZooUpdaterService", "Could not reopen Zoo after installation", failure)
        }
    }

}


/**
 * Shizuku のシェルのプロセスはアプリの辞書を読めないので、失敗はこの接頭辞の付いた文字列で返す。
 * アプリ側（UpdateWorker）が接頭辞を外し、`android.update.installFailed` で訳して表示する。
 */
internal const val INSTALL_FAILED_PREFIX = "インストールできませんでした: "

internal fun completeInstall(exitCode: Int, output: String, restart: () -> Unit): String {
    if (exitCode != 0 || output.lineSequence().none { it.trim() == "Success" }) {
        return "$INSTALL_FAILED_PREFIX${output.take(1000)}"
    }
    restart()
    return "Success"
}

internal fun restartCommand(userId: Int): List<String> = listOf(
    "/system/bin/am", "start", "--user", userId.toString(),
    "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER",
    "-n", "works.zli.zoo/zoo.android.MainActivity",
)
