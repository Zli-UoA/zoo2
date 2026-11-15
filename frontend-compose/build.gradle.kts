import java.security.MessageDigest
import java.net.URI
import java.io.IOException

plugins {
    id("com.android.library") version "8.11.1" apply false
    id("com.android.application") version "8.11.1" apply false
    kotlin("multiplatform") version "2.3.20"
    kotlin("plugin.serialization") version "2.3.20"
    id("org.jetbrains.compose") version "1.11.1"
    id("org.jetbrains.kotlin.plugin.compose") version "2.3.20"
}

val androidEnabled = providers.gradleProperty("zoo.android").orNull == "true"
if (androidEnabled) apply(plugin = "com.android.library")
val jnaVersion = "5.17.0"

kotlin {
    compilerOptions { optIn.addAll("kotlin.time.ExperimentalTime") }
    if (androidEnabled) androidTarget { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
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
        if (androidEnabled) named("androidMain") {
            dependencies {
                implementation("io.ktor:ktor-client-okhttp:3.1.3"); implementation("com.squareup.okhttp3:okhttp:4.12.0")
                // zoo-crypto の UniFFI バインディング（#353）が使う。Android の各 ABI の libjnidispatch.so は aar に入っている。
                implementation("net.java.dev.jna:jna:$jnaVersion@aar")
            }
        }
        // 単体テストは JVM で動くので、ホストの libjnidispatch を持つ jar も入れる。
        if (androidEnabled) named("androidUnitTest") { dependencies { implementation("net.java.dev.jna:jna:$jnaVersion") } }
        commonTest.dependencies {
            implementation(kotlin("test"))
            implementation("io.ktor:ktor-client-mock:3.1.3")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
        }
    }
}

// Electric4k currently uses kotlinx.datetime.Clock/Instant. Keep the compatibility
// artifact consistent across the app and upstream module.
allprojects { configurations.configureEach { resolutionStrategy.force(
    "org.jetbrains.kotlinx:kotlinx-datetime:0.7.1-0.6.x-compat",
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

if (androidEnabled) extensions.configure<com.android.build.gradle.LibraryExtension> {
    namespace = "works.zli.zoo.shared"
    compileSdk = 35
    defaultConfig { minSdk = 26 }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}

// zoo-crypto の UniFFI バインディング（crates/zoo-crypto-ffi、#353）。
// - Android の各 ABI の .so は cargo-ndk で作り、jniLibs に入れる。
// - Kotlin のバインディングは、ホスト向けにビルドした .so から uniffi-bindgen で作る。
// - 単体テスト（JVM）は、同じホストの .so を JNA で読む。
// cargo と cargo-ndk、ANDROID_NDK_HOME が要る（`nix develop .#android`、CI Android と同じ）。
if (androidEnabled) {
    val cryptoCrate = rootDir.resolve("../crates/zoo-crypto-ffi")
    val cryptoSources = files(
        rootDir.resolve("../crates/zoo-crypto/src"),
        rootDir.resolve("../crates/zoo-crypto/Cargo.toml"),
        cryptoCrate.resolve("src"),
        cryptoCrate.resolve("Cargo.toml"),
        cryptoCrate.resolve("Cargo.lock"),
        cryptoCrate.resolve("uniffi.toml"),
        rootDir.resolve("../rust-toolchain.toml"),
    )
    // cargo の出力先は crates/zoo-crypto-ffi/target（cargo の既定）。ホストの .so はそのまま JNA に読ませる。
    val hostLibraryDir = cryptoCrate.resolve("target/debug")
    val hostLibrary = hostLibraryDir.resolve(
        System.mapLibraryName("zoo_crypto_ffi"),
    )
    val buildCryptoHost by tasks.registering(Exec::class) {
        group = "zoo crypto"
        description = "Build zoo-crypto-ffi for the host (bindgen and JVM unit tests)."
        inputs.files(cryptoSources)
        outputs.file(hostLibrary)
        workingDir(cryptoCrate)
        commandLine("cargo", "build", "--locked", "--lib")
    }
    val cryptoBindingsDir = layout.buildDirectory.dir("generated/uniffi/kotlin")
    val generateCryptoBindings by tasks.registering(Exec::class) {
        group = "zoo crypto"
        description = "Generate the Kotlin bindings of zoo-crypto-ffi with uniffi-bindgen."
        dependsOn(buildCryptoHost)
        inputs.files(cryptoSources)
        inputs.file(hostLibrary)
        outputs.dir(cryptoBindingsDir)
        workingDir(cryptoCrate)
        doFirst { cryptoBindingsDir.get().asFile.deleteRecursively() }
        commandLine(
            "cargo", "run", "--locked", "--quiet", "--features", "cli", "--bin", "uniffi-bindgen", "--",
            "generate", hostLibrary.absolutePath, "--language", "kotlin", "--no-format",
            "--out-dir", cryptoBindingsDir.get().asFile.absolutePath,
        )
    }
    kotlin.sourceSets.named("androidMain") { kotlin.srcDir(generateCryptoBindings) }

    // docs/e2ee.md の「共有の暗号コア」の【決定】と同じ ABI。flake.nix の androidRustTargets と合わせる。
    val cryptoAbis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")
    val cryptoJniLibsDir = layout.buildDirectory.dir("generated/uniffi/jniLibs")
    val buildCryptoAndroid by tasks.registering(Exec::class) {
        group = "zoo crypto"
        description = "Build zoo-crypto-ffi for the Android ABIs with cargo-ndk."
        inputs.files(cryptoSources)
        outputs.dir(cryptoJniLibsDir)
        workingDir(cryptoCrate)
        doFirst { cryptoJniLibsDir.get().asFile.deleteRecursively() }
        commandLine(
            listOf("cargo", "ndk") + cryptoAbis.flatMap { listOf("-t", it) } +
                listOf("-P", "26", "-o", cryptoJniLibsDir.get().asFile.absolutePath, "build", "--locked", "--release", "--lib"),
        )
    }
    extensions.configure<com.android.build.gradle.LibraryExtension> {
        sourceSets.getByName("main").jniLibs.srcDir(cryptoJniLibsDir)
    }
    tasks.named("preBuild") { dependsOn(buildCryptoAndroid) }
    tasks.withType<Test>().configureEach {
        dependsOn(buildCryptoHost)
        inputs.file(hostLibrary)
        systemProperty("jna.library.path", hostLibraryDir.absolutePath)
    }
}
