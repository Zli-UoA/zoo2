@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.io.encoding.Base64
import kotlin.js.*

// SVG geometry/style comes from Kotlin. This browser boundary only decodes an
// image and returns its pixels, just like the image-loading platform boundary.
@JsFun(
    """async (svg, width, height) => {
    globalThis.__zooPendingImageDecodes = (globalThis.__zooPendingImageDecodes ?? 0) + 1;
    let source;
    try {
        source = URL.createObjectURL(new Blob([svg], {type:'image/svg+xml'}));
        const image = new Image();
        image.src = source;
        await image.decode();
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        canvas.getContext('2d').drawImage(image, 0, 0, width, height);
        return canvas.toDataURL('image/png').split(',')[1];
    } finally {
        if (source) URL.revokeObjectURL(source);
        globalThis.__zooPendingImageDecodes--;
    }
}"""
)
private external fun rasterizeSvgPixels(svg: String, width: Int, height: Int): Promise<JsString>

internal suspend fun browserRasterizeSvg(svg: String, width: Int, height: Int): ByteArray =
    Base64.decode(rasterizeSvgPixels(svg, width, height).awaitBrowser().toString())
