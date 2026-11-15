package zoo.android.update

import android.content.ComponentName
import android.content.Context
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.util.Log
import java.io.File
import kotlinx.coroutines.*
import rikka.shizuku.Shizuku

internal fun shizukuReady(): Boolean =
    try {
        Shizuku.pingBinder() &&
            !Shizuku.isPreV11() &&
            Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
    } catch (_: Exception) {
        false
    }

internal class ShizukuInstaller(private val context: Context) {
    suspend fun install(file: File): String {
        check(shizukuReady()) { "Shizuku を起動し、Zoo に使用を許可してください" }
        val connected = CompletableDeferred<IUpdateInstaller>()
        val args =
            Shizuku.UserServiceArgs(
                    ComponentName(context.packageName, UpdateInstallerService::class.java.name)
                )
                .daemon(true)
                .processNameSuffix("updater")
                .version(works.zli.zoo.BuildConfig.VERSION_CODE)
        val connection =
            object : ServiceConnection {
                override fun onServiceConnected(name: ComponentName, binder: IBinder) {
                    try {
                        val descriptor = binder.interfaceDescriptor
                        check(descriptor == IUpdateInstaller::class.java.name) {
                            "Shizuku のインストーラーの接続先が一致しません: $descriptor"
                        }
                        Log.i(
                            "ZooUpdater",
                            "Installer connected: $descriptor, alive=${binder.isBinderAlive}",
                        )
                        connected.complete(IUpdateInstaller.Stub.asInterface(binder))
                    } catch (failure: Exception) {
                        connected.completeExceptionally(failure)
                    }
                }

                override fun onServiceDisconnected(name: ComponentName) {
                    connected.completeExceptionally(IllegalStateException("Shizuku との接続が切れました"))
                }
            }
        return installerSession(
            connect = {
                withContext(Dispatchers.Main) { Shizuku.bindUserService(args, connection) }
                withTimeoutOrNull(20_000) { connected.await() }
                    ?: error("Shizuku のインストーラーに接続できませんでした。Shizuku を再起動して再試行してください")
            },
            install = { installer ->
                withContext(Dispatchers.IO) {
                    ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use {
                        // The service has its own watchdog; Binder calls themselves are not
                        // cancellable.
                        val result =
                            installer.install(it, file.length(), android.os.Process.myUid() / 100000)
                        if (result.isNullOrBlank()) {
                            val server = runCatching {
                                "Shizuku=${Shizuku.getVersion()}, uid=${Shizuku.getUid()}"
                            }.getOrDefault("Shizuku disconnected")
                            Log.e(
                                "ZooUpdater",
                                "Installer returned an empty response; Android=${android.os.Build.VERSION.SDK_INT}, $server",
                            )
                        }
                        result
                    }
                }
            },
            disconnect = {
                withContext(Dispatchers.Main) { Shizuku.unbindUserService(args, connection, true) }
            },
        )
    }
}

internal suspend fun awaitShizukuReady(): Boolean {
    var listener: Shizuku.OnBinderReceivedListener? = null
    val alive =
        awaitServiceBinder(
            isAlive = { Shizuku.pingBinder() },
            subscribe = { received ->
                listener = Shizuku.OnBinderReceivedListener { received() }
                Shizuku.addBinderReceivedListenerSticky(requireNotNull(listener))
            },
            unsubscribe = {
                listener?.let { Shizuku.removeBinderReceivedListener(it) }
            },
        )
    return alive && shizukuReady()
}
