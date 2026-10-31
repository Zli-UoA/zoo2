import java.security.MessageDigest

plugins {
    kotlin("multiplatform")
    kotlin("plugin.serialization")
}

// Resolve a public, immutable source archive through Gradle's normal dependency cache.
// No library source, submodule or local Maven publication is committed to this repository.
val upstreamCommit = "489e32f85fd608e7e6a883e2f044d5edad8a7c44"
val upstreamSha256 = "94ead69b6bbef7941d4c811a55f2a42dba41123f518e4fff1b268e685f4e399d"
val upstreamArchive by configurations.creating {
    isCanBeConsumed = false
    isCanBeResolved = true
    isTransitive = false
}
dependencies {
    upstreamArchive("io.github.kai-z-jp:electric4k-source:$upstreamCommit@tar.gz")
}
val unpackUpstream by tasks.registering(Sync::class) {
    inputs.property("upstreamCommit", upstreamCommit)
    inputs.property("upstreamSha256", upstreamSha256)
    from(provider {
        val archive = upstreamArchive.singleFile
        val digest = MessageDigest.getInstance("SHA-256")
        archive.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var length = input.read(buffer)
            while (length >= 0) {
                digest.update(buffer, 0, length)
                length = input.read(buffer)
            }
        }
        val actual = digest.digest().joinToString("") { "%02x".format(it) }
        check(actual == upstreamSha256) { "Electric4k archive checksum does not match the pinned commit" }
        tarTree(resources.gzip(archive))
    }) {
        eachFile { relativePath = RelativePath(true, *relativePath.segments.drop(1).toTypedArray()) }
        includeEmptyDirs = false
        include("*/src/commonMain/**", "*/src/wasmJsMain/**", "*/LICENSE", "*/NOTICE")
    }
    into(layout.buildDirectory.dir("upstream"))
}

kotlin {
    compilerOptions { optIn.addAll("kotlin.time.ExperimentalTime", "kotlin.js.ExperimentalWasmJsInterop") }
    @OptIn(org.jetbrains.kotlin.gradle.ExperimentalWasmDsl::class)
    wasmJs { browser() }
    sourceSets {
        commonMain {
            kotlin.srcDir(unpackUpstream.map { it.destinationDir.resolve("src/commonMain/kotlin") })
            dependencies {
                api("io.ktor:ktor-client-core:3.1.3")
                api("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
                api("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.7.1-0.6.x-compat")
            }
        }
        wasmJsMain {
            kotlin.srcDir(unpackUpstream.map { it.destinationDir.resolve("src/wasmJsMain/kotlin") })
            dependencies { implementation("io.ktor:ktor-client-js:3.1.3") }
        }
    }
}

// Preserve the upstream license/notice in the binary Web distribution.
rootProject.tasks.named<Copy>("wasmJsProcessResources") {
    from(unpackUpstream) {
        include("LICENSE", "NOTICE")
        into("licenses/electric4k")
    }
}
