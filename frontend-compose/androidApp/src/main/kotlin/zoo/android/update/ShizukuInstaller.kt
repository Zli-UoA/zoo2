package zoo.android.update

import android.content.ComponentName
import android.content.Context
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.IBinder
import android.os.ParcelFileDescriptor
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
    suspend fun install(file: File): String = coroutineScope {
        check(shizukuReady()) { "Shizuku を起動し、Zoo に使用を許可してください" }
        val connected = CompletableDeferred<IUpdateInstaller>()
        val args =
            Shizuku.UserServiceArgs(
                    ComponentName(context.packageName, UpdateInstallerService::class.java.name)
                )
                .daemon(false)
                .processNameSuffix("updater")
                .version(works.zli.zoo.BuildConfig.VERSION_CODE)
        val connection =
            object : ServiceConnection {
                override fun onServiceConnected(name: ComponentName, binder: IBinder) {
                    connected.complete(IUpdateInstaller.Stub.asInterface(binder))
                }

                override fun onServiceDisconnected(name: ComponentName) {
                    connected.completeExceptionally(IllegalStateException("Shizuku との接続が切れました"))
                }
            }
        try {
            withContext(Dispatchers.Main) { Shizuku.bindUserService(args, connection) }
            val installer = withTimeout(20_000) { connected.await() }
            withContext(Dispatchers.IO) {
                ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use {
                    // The service has its own watchdog; Binder calls themselves are not
                    // cancellable.
                    installer.install(it, file.length(), android.os.Process.myUid() / 100000)
                }
            }
        } finally {
            withContext(NonCancellable + Dispatchers.Main) {
                runCatching { Shizuku.unbindUserService(args, connection, true) }
            }
        }
    }
}
