package zoo.android

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import java.util.concurrent.Executors
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

/** カメラの状態（Web の `e2ee.qr.camera.*` と同じ区別）。 */
internal enum class CameraState { STARTING, RUNNING, DENIED, NO_CAMERA, FAILED }

/**
 * 相手の画面の検証の QR をカメラで読み取る（#567）。読み取れたバイト列を `onCode` に渡す。`onCode` が `false` を
 * 返したら（今の検証の QR でない）、読み取りを続ける。画面を離れたらカメラを止める。
 */
@Composable
internal fun QrScanner(
    label: String,
    onCode: suspend (ByteArray) -> Boolean,
    onState: (CameraState) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        granted = allowed
        if (!allowed) onState(CameraState.DENIED)
    }
    LaunchedEffect(Unit) { if (!granted) permission.launch(Manifest.permission.CAMERA) }
    if (!granted) return
    val latestOnCode by rememberUpdatedState(onCode)
    val scope = rememberCoroutineScope()
    val executor = remember { Executors.newSingleThreadExecutor() }
    // 1 つずつ確かめる（確かめているあいだの映像は捨てる）。
    val checking = remember { AtomicBoolean(false) }
    // 画面から外したか（初期化が終わる前に外したら、あとから届いた初期化の通知でカメラを bind しない）と、bind した
    // provider（外すときに、初期化を待たずに unbind する）。どちらも main thread だけで読み書きする。
    val released = remember { AtomicBoolean(false) }
    var bound by remember { mutableStateOf<ProcessCameraProvider?>(null) }
    DisposableEffect(Unit) { onDispose { executor.shutdown() } }
    AndroidView(
        factory = { viewContext ->
            val preview = PreviewView(viewContext)
            val future = ProcessCameraProvider.getInstance(viewContext)
            future.addListener({
                if (released.get()) return@addListener
                val provider = try { future.get() } catch (_: Exception) { onState(CameraState.FAILED); return@addListener }
                if (!provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA) && !provider.hasCamera(CameraSelector.DEFAULT_FRONT_CAMERA)) {
                    onState(CameraState.NO_CAMERA)
                    return@addListener
                }
                val selector = if (provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA)) CameraSelector.DEFAULT_BACK_CAMERA
                    else CameraSelector.DEFAULT_FRONT_CAMERA
                val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                analysis.setAnalyzer(executor) { image ->
                    try {
                        if (checking.get()) return@setAnalyzer
                        val plane = image.planes[0]
                        val buffer = plane.buffer
                        val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
                        val code = decodeQr(bytes, image.width, image.height, plane.rowStride) ?: return@setAnalyzer
                        checking.set(true)
                        scope.launch {
                            try { if (latestOnCode(code)) return@launch } finally { checking.set(false) }
                        }
                    } catch (_: Exception) {
                        // 読み取れない映像は捨てて、次の映像を待つ。
                    } finally {
                        image.close()
                    }
                }
                val previewUse = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
                try {
                    provider.unbindAll()
                    provider.bindToLifecycle(lifecycle, selector, previewUse, analysis)
                    bound = provider
                    onState(CameraState.RUNNING)
                } catch (_: Exception) {
                    onState(CameraState.FAILED)
                }
            }, ContextCompat.getMainExecutor(viewContext))
            preview
        },
        onRelease = {
            released.set(true)
            // 初期化の完了を待たない（UI thread を止めないため）。bind 済みのときだけ外す。
            try { bound?.unbindAll() } catch (_: Exception) { }
            bound = null
        },
        modifier = modifier.fillMaxWidth().aspectRatio(1f).semantics { contentDescription = label },
    )
}
