import java.security.MessageDigest
import java.net.URI
import java.io.IOException
import org.jetbrains.kotlin.gradle.targets.wasm.nodejs.WasmNodeJsEnvSpec

plugins {
    kotlin("multiplatform") version "2.2.20"
    kotlin("plugin.serialization") version "2.2.20"
    id("org.jetbrains.compose") version "1.9.3"
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.20"
}

kotlin {
    compilerOptions { optIn.addAll("kotlin.time.ExperimentalTime", "kotlin.js.ExperimentalWasmJsInterop") }
    @OptIn(org.jetbrains.kotlin.gradle.ExperimentalWasmDsl::class)
    wasmJs {
        outputModuleName.set("zoo-compose")
        browser {
            commonWebpackConfig {
                outputFileName = "zoo-compose.js"
            }
            testTask { useKarma { useChromeHeadless() } }
        }
        binaries.executable()
    }
    sourceSets {
        commonMain.dependencies {
            implementation(project(":electric4k"))
            implementation(compose.runtime)
            implementation(compose.foundation)
            implementation(compose.material3)
            implementation(compose.components.resources)
            implementation("io.ktor:ktor-client-core:3.1.3")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
            implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.7.1-0.6.x-compat")
        }
        wasmJsMain.dependencies {
            implementation("io.ktor:ktor-client-js:3.1.3")
            implementation("io.ktor:ktor-client-mock:3.1.3")
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
            implementation("io.ktor:ktor-client-mock:3.1.3")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
        }
    }
}

rootProject.plugins.withType<org.jetbrains.kotlin.gradle.targets.wasm.nodejs.WasmNodeJsRootPlugin> {
    rootProject.extensions.configure<WasmNodeJsEnvSpec> {
        if (providers.gradleProperty("zoo.systemNode").orNull == "true") download.set(false)
    }
}

// Electric4k currently uses kotlinx.datetime.Clock/Instant. Keep the compatibility
// artifact consistent across the app, upstream module and browser test linker.
allprojects { configurations.configureEach { resolutionStrategy.force(
    "org.jetbrains.kotlinx:kotlinx-datetime:0.7.1-0.6.x-compat",
    "org.jetbrains.kotlinx:kotlinx-datetime-wasm-js:0.7.1-0.6.x-compat",
) } }

// Fonts are downloaded into build/, never checked in. Both archives and extracted
// font bytes are pinned; a checksum mismatch fails the build before resource generation.
val fontOutput = layout.buildDirectory.dir("downloadedFonts")
val downloadComposeFonts by tasks.registering {
    inputs.file("fonts.lock")
    outputs.dir(fontOutput)
    doLast {
        fun sha(file: File): String {
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { stream ->
                val buffer = ByteArray(8192)
                var length = stream.read(buffer)
                while (length >= 0) {
                    digest.update(buffer, 0, length)
                    length = stream.read(buffer)
                }
            }
            return digest.digest().joinToString("") { "%02x".format(it) }
        }
        val downloads = layout.buildDirectory.dir("fontDownloads").get().asFile.apply { mkdirs() }
        val output = fontOutput.get().asFile.apply { mkdirs() }
        file("fonts.lock").readLines().filter { it.isNotBlank() && !it.startsWith("#") }.forEach { line ->
            val (name, checksum, url, member, fontChecksum) = line.split('|')
            val target = output.resolve(name)
            if (target.isFile && sha(target) == fontChecksum) return@forEach
            val source = downloads.resolve(checksum)
            if (!source.isFile || sha(source) != checksum) {
                val pending = downloads.resolve("$checksum.part")
                for (attempt in 1..3) {
                    try {
                        val connection = URI(url).toURL().openConnection().apply {
                            connectTimeout = 30_000
                            readTimeout = 60_000
                        }
                        connection.getInputStream().use { input -> pending.outputStream().use { input.copyTo(it) } }
                        break
                    } catch (error: IOException) {
                        pending.delete()
                        if (attempt == 3) throw error
                    }
                }
                check(sha(pending) == checksum) { "Font source checksum mismatch: $name" }
                pending.copyTo(source, overwrite = true)
                pending.delete()
            }
            val font = if (member == "-") source else
                tarTree(resources.gzip(source)).matching { include(member) }.singleFile
            check(sha(font) == fontChecksum) { "Font checksum mismatch: $name" }
            font.copyTo(target, overwrite = true)
        }
    }
}
apply(from = "gradle/font-tools.gradle.kts")
kotlin.sourceSets.named("commonMain") {
    kotlin.srcDir(tasks.named("generateComposeFontCoverage"))
}

val normalizedFontOutput = layout.buildDirectory.dir("normalizedFonts")
val catalogResources by tasks.registering(Sync::class) {
    dependsOn(tasks.named("normalizeComposeFonts"))
    from("src/commonMain/composeResources")
    from(normalizedFontOutput) {
        include(file("fonts.lock").readLines().filter { it.isNotBlank() && !it.startsWith("#") }.map { it.substringBefore('|') })
        into("font")
    }
    into(layout.buildDirectory.dir("generated/composeResources"))
}
compose.resources {
    customDirectory("commonMain", catalogResources.map { layout.buildDirectory.dir("generated/composeResources").get() })
}
