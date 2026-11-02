import java.io.File

// Read the pinned SFNT cmap rather than maintaining a second Unicode/font allowlist.
object ComposeFontTools {
    fun u16(bytes: ByteArray, offset: Int): Int =
        ((bytes[offset].toInt() and 255) shl 8) or (bytes[offset + 1].toInt() and 255)

    fun u32(bytes: ByteArray, offset: Int): Long =
        (0..3).fold(0L) { result, index -> (result shl 8) or (bytes[offset + index].toLong() and 255L) }

    fun table(bytes: ByteArray, tag: String): Pair<Int, Int> {
        val count = u16(bytes, 4)
        for (index in 0 until count) {
            val record = 12 + index * 16
            if (String(bytes, record, 4, Charsets.US_ASCII) == tag) {
                val offset = u32(bytes, record + 8).toInt()
                val length = u32(bytes, record + 12).toInt()
                check(offset >= 0 && length >= 0 && offset.toLong() + length <= bytes.size)
                return offset to length
            }
        }
        error("Missing SFNT table $tag")
    }

    fun put16(bytes: ByteArray, offset: Int, value: Int) {
        bytes[offset] = (value ushr 8).toByte()
        bytes[offset + 1] = value.toByte()
    }

    fun put32(bytes: ByteArray, offset: Int, value: Long) {
        for (index in 0..3) bytes[offset + index] = (value ushr (24 - index * 8)).toByte()
    }

    fun checksum(bytes: ByteArray, offset: Int = 0, length: Int = bytes.size): Long {
        var result = 0L
        for (relative in 0 until length step 4) {
            var word = 0L
            for (index in 0..3) word = (word shl 8) or
                (if (relative + index < length) bytes[offset + relative + index].toLong() and 255L else 0L)
            result = (result + word) and 0xffffffffL
        }
        return result
    }

    fun normalizeFallbackMetrics(source: ByteArray, primary: ByteArray): ByteArray {
        val bytes = source.copyOf()
        val units = u16(source, table(source, "head").first + 18)
        val primaryUnits = u16(primary, table(primary, "head").first + 18)
        fun scaled(value: Int) = kotlin.math.round(value.toDouble() * units / primaryUnits).toInt()
        fun signed(data: ByteArray, offset: Int) = u16(data, offset).toShort().toInt()
        val hhea = table(bytes, "hhea").first
        val primaryHhea = table(primary, "hhea").first
        for (relative in listOf(4, 6, 8)) put16(bytes, hhea + relative, scaled(signed(primary, primaryHhea + relative)))
        val os2 = table(bytes, "OS/2").first
        val primaryOs2 = table(primary, "OS/2").first
        for (relative in listOf(68, 70, 72)) put16(bytes, os2 + relative, scaled(signed(primary, primaryOs2 + relative)))
        for (relative in listOf(74, 76)) put16(bytes, os2 + relative, scaled(u16(primary, primaryOs2 + relative)))
        put16(bytes, os2 + 62, (u16(bytes, os2 + 62) and 128.inv()) or (u16(primary, primaryOs2 + 62) and 128))

        // Identify the derived family without changing the CFF/PostScript glyph identity.
        // Same-length UTF-16 replacements preserve every table offset and copyright/license record.
        val name = table(bytes, "name").first
        check(u16(bytes, name) == 0) { "Unsupported SFNT name format" }
        val strings = name + u16(bytes, name + 4)
        var renamed = false
        for (index in 0 until u16(bytes, name + 2)) {
            val record = name + 6 + index * 12
            if (u16(bytes, record) != 3 || u16(bytes, record + 6) !in listOf(1, 3, 4, 16)) continue
            val length = u16(bytes, record + 8)
            val offset = strings + u16(bytes, record + 10)
            val original = String(bytes, offset, length, Charsets.UTF_16BE)
            val replacement = original.replace("Noto", "ZooC").toByteArray(Charsets.UTF_16BE)
            check(replacement.size == length)
            renamed = renamed || original.contains("Noto")
            replacement.copyInto(bytes, offset)
        }
        check(renamed) { "Expected pinned Noto family name" }
        val head = table(bytes, "head").first
        put32(bytes, head + 8, 0L)
        for (index in 0 until u16(bytes, 4)) {
            val record = 12 + index * 16
            put32(bytes, record + 4, checksum(bytes, u32(bytes, record + 8).toInt(), u32(bytes, record + 12).toInt()))
        }
        put32(bytes, head + 8, (0xb1b0afbaL - checksum(bytes)) and 0xffffffffL)
        check(checksum(bytes) == 0xb1b0afbaL) { "Invalid normalized SFNT checksum" }
        for (tag in listOf("CFF ", "hmtx", "vmtx", "cmap")) {
            val (offset, length) = table(source, tag)
            check(source.copyOfRange(offset, offset + length).contentEquals(bytes.copyOfRange(offset, offset + length))) {
                "Normalization changed glyph outlines, mapping or advances: $tag"
            }
        }
        return bytes
    }

    fun unicodeRanges(bytes: ByteArray): List<Int> {
        val (cmap, _) = table(bytes, "cmap")
        val covered = BooleanArray(0x110000)
        for (index in 0 until u16(bytes, cmap + 2)) {
            val record = cmap + 4 + index * 8
            val platform = u16(bytes, record)
            val encoding = u16(bytes, record + 2)
            if (platform != 0 && !(platform == 3 && encoding in listOf(1, 10))) continue
            val subtable = cmap + u32(bytes, record + 4).toInt()
            when (u16(bytes, subtable)) {
                4 -> {
                    val segments = u16(bytes, subtable + 6) / 2
                    val ends = subtable + 14
                    val starts = ends + segments * 2 + 2
                    val deltas = starts + segments * 2
                    val offsets = deltas + segments * 2
                    for (segment in 0 until segments) {
                        val start = u16(bytes, starts + segment * 2)
                        val end = u16(bytes, ends + segment * 2)
                        val delta = u16(bytes, deltas + segment * 2)
                        val offsetField = offsets + segment * 2
                        val rangeOffset = u16(bytes, offsetField)
                        for (point in start..minOf(end, 0xfffe)) {
                            val raw = if (rangeOffset == 0) point else
                                u16(bytes, offsetField + rangeOffset + (point - start) * 2)
                            val glyph = if (rangeOffset != 0 && raw == 0) 0 else (raw + delta) and 0xffff
                            if (glyph != 0) covered[point] = true
                        }
                    }
                }
                12, 13 -> {
                    val format = u16(bytes, subtable)
                    for (group in 0 until u32(bytes, subtable + 12).toInt()) {
                        val offset = subtable + 16 + group * 12
                        val start = u32(bytes, offset).toInt()
                        val end = minOf(u32(bytes, offset + 4).toInt(), covered.lastIndex)
                        val glyph = u32(bytes, offset + 8)
                        for (point in start..end) {
                            if (glyph != 0L || format == 12 && point > start) covered[point] = true
                        }
                    }
                }
            }
        }
        val ranges = mutableListOf<Int>()
        var start = -1
        for (point in 0..covered.size) {
            val present = point < covered.size && covered[point]
            if (present && start < 0) start = point
            if (!present && start >= 0) {
                ranges.add(start)
                ranges.add(point - 1)
                start = -1
            }
        }
        check(ranges.isNotEmpty()) { "Font has no Unicode cmap" }
        return ranges
    }
}

val coverageOutput = layout.buildDirectory.dir("generated/fontCoverage")
val generateComposeFontCoverage by tasks.registering {
    dependsOn("downloadComposeFonts")
    inputs.file("fonts.lock")
    inputs.file("gradle/font-tools.gradle.kts")
    inputs.files(layout.buildDirectory.file("downloadedFonts/Geist.ttf"), layout.buildDirectory.file("downloadedFonts/NotoColorEmoji.ttf"))
    outputs.dir(coverageOutput)
    doLast {
        fun ranges(name: String) = ComposeFontTools.unicodeRanges(layout.buildDirectory.file("downloadedFonts/$name").get().asFile.readBytes())
        fun literal(values: List<Int>) = values.joinToString(", ") { "0x${it.toString(16)}" }
        val output = coverageOutput.get().file("zoo/CssFontCoverage.kt").asFile
        output.parentFile.mkdirs()
        output.writeText("""
            // Generated from fonts.lock source cmaps. Do not edit.
            package zoo

            internal object CssFontCoverage {
                private val latin = intArrayOf(${literal(ranges("Geist.ttf"))})
                private val emoji = intArrayOf(${literal(ranges("NotoColorEmoji.ttf"))})
                fun latinContains(point: Int) = contains(latin, point)
                fun emojiContains(point: Int) = contains(emoji, point)
                private fun contains(ranges: IntArray, point: Int): Boolean {
                    var low = 0
                    var high = ranges.size / 2 - 1
                    while (low <= high) {
                        val middle = (low + high) ushr 1
                        if (point < ranges[middle * 2]) high = middle - 1
                        else if (point > ranges[middle * 2 + 1]) low = middle + 1
                        else return true
                    }
                    return false
                }
            }
        """.trimIndent() + "\n")
    }
}


val normalizedFontOutput = layout.buildDirectory.dir("normalizedFonts")
val normalizeComposeFonts by tasks.registering {
    dependsOn("downloadComposeFonts")
    inputs.file("fonts.lock")
    inputs.file("gradle/font-tools.gradle.kts")
    inputs.dir(layout.buildDirectory.dir("downloadedFonts"))
    outputs.dir(normalizedFontOutput)
    doLast {
        val source = layout.buildDirectory.dir("downloadedFonts").get().asFile
        val primary = source.resolve("Geist.ttf").readBytes()
        val output = normalizedFontOutput.get().asFile.apply { mkdirs() }
        file("fonts.lock").readLines().filter { it.isNotBlank() && !it.startsWith("#") }.forEach { line ->
            val name = line.substringBefore('|')
            val bytes = source.resolve(name).readBytes()
            output.resolve(name).writeBytes(
                if (name in listOf("NotoSansJPRegular.otf", "NotoSansJPBold.otf"))
                    ComposeFontTools.normalizeFallbackMetrics(bytes, primary)
                else bytes
            )
        }
    }
}
