package zoo.android.update

import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.annotation.Keep
import java.io.IOException
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
            if (process.exitValue() == 0 && result.lineSequence().any { it == "Success" }) "Success"
            else "インストールできませんでした: ${result.take(1000)}"
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
            "インストールできませんでした: ${result.ifBlank { failure.toString() }} (exit=$exit)"
        } catch (failure: Exception) {
            Log.e("ZooUpdaterService", "Install failed", failure)
            throw failure
        } finally {
            process.destroy()
            watchdog.interrupt()
        }
    }
}
