import java.util.Properties

plugins {
    id("com.android.application")
    kotlin("android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "works.zli.zoo"
    compileSdk = 36
    defaultConfig {
        applicationId = "works.zli.zoo"
        minSdk = 26
        targetSdk = 35
        versionCode = providers.gradleProperty("zoo.android.versionCode").orNull?.toInt() ?: 1
        versionName = providers.gradleProperty("zoo.android.versionName").orNull ?: "0.1.0"
        // E2EE の再認証（#360）のリダイレクト先のホスト。App Links の intent-filter（manifest）と、
        // 認可リクエストの redirect_uri（AndroidReauth）の両方にこの値を使う。このホストの nginx が
        // assetlinks.json を配り、AuthZ の Android のクライアントに https://<ホスト>/android/reauth-callback を登録する。
        val reauthHost = providers.gradleProperty("zoo.android.reauthHost").orNull ?: "zli.chat"
        require(Regex("[a-z0-9.-]+").matches(reauthHost)) { "zoo.android.reauthHost はホスト名にしてください" }
        manifestPlaceholders["reauthHost"] = reauthHost
        buildConfigField("String", "REAUTH_HOST", "\"$reauthHost\"")
        // Firebase's public Android identifiers; service-account credentials belong on the server.
        val firebaseDefaults = Properties().apply {
            val config = file("firebase-public.properties")
            if (config.isFile) config.inputStream().use { load(it) }
        }
        val firebaseKeys = mapOf(
            "google_app_id" to "APP_ID",
            "google_api_key" to "API_KEY",
            "gcm_defaultSenderId" to "SENDER_ID",
            "project_id" to "PROJECT_ID",
        )
        val firebaseEnvironment = firebaseKeys.mapValues { (_, key) ->
            providers.environmentVariable("ZOO_ANDROID_FIREBASE_$key").orNull.orEmpty()
        }
        require(firebaseEnvironment.values.all { it.isBlank() } || firebaseEnvironment.values.all { it.isNotBlank() }) {
            "Configure all four ZOO_ANDROID_FIREBASE_* environment overrides, or leave all unset"
        }
        val firebase = firebaseKeys.mapValues { (name, key) ->
            if (firebaseEnvironment.values.any { it.isNotBlank() }) firebaseEnvironment.getValue(name)
            else firebaseDefaults.getProperty("ZOO_ANDROID_FIREBASE_$key").orEmpty()
        }
        require(firebase.values.all { it.isEmpty() } || firebase.values.all { it.isNotEmpty() }) {
            "Configure all four ZOO_ANDROID_FIREBASE_* identifiers, or leave all unset"
        }
        firebase.forEach { (name, value) -> resValue("string", name, value) }
    }
    buildFeatures {
        aidl = true
        compose = true
        buildConfig = true
    }
    val signingPath = providers.environmentVariable("ZOO_ANDROID_KEYSTORE").orNull
    if (signingPath != null) {
        signingConfigs.create("distribution") {
            storeFile = file(signingPath)
            storePassword = providers.environmentVariable("ZOO_ANDROID_STORE_PASSWORD").get()
            keyAlias = providers.environmentVariable("ZOO_ANDROID_KEY_ALIAS").get()
            keyPassword = providers.environmentVariable("ZOO_ANDROID_KEY_PASSWORD").get()
        }
        buildTypes.getByName("release").signingConfig = signingConfigs.getByName("distribution")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
}

dependencies {
    implementation(project(":"))
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")
    implementation("androidx.work:work-runtime-ktx:2.10.1")
    implementation("com.google.firebase:firebase-messaging:24.1.1")
    implementation("com.caverock:androidsvg-aar:1.4")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.exifinterface:exifinterface:1.4.1")
    implementation("org.jetbrains.compose.material3:material3:1.9.0")
    implementation("io.ktor:ktor-client-okhttp:3.1.3")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")
    // E2EE の初回設定と解錠（#360）。パスキーの PRF は Credential Manager、再認証は Custom Tabs。
    implementation("androidx.credentials:credentials:1.5.0")
    implementation("androidx.credentials:credentials-play-services-auth:1.5.0")
    implementation("androidx.browser:browser:1.9.0")
    // 他のユーザーの検証の QR（#567）：描画と読み取り（ZXing）と、読み取りのカメラ（CameraX）。
    implementation("com.google.zxing:core:3.5.3")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
}

val translations by
    tasks.registering(Sync::class) {
        from("../../frontend/src/lib/i18n/locales")
        into(layout.buildDirectory.dir("generated/assets/locales"))
    }

val emojiCatalogs by tasks.registering(Sync::class) {
    from("../src/commonMain/composeResources/files") {
        include("shortcodes.json", "emoji-kitchen.json")
    }
    from("../../frontend/public/emojibase-data") {
        include("*/data.json", "*/messages.json", "LICENSE")
    }
    into(layout.buildDirectory.dir("generated/assets/emoji"))
}

android.sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/assets"))

tasks.named("preBuild") { dependsOn(translations, emojiCatalogs) }

dependencies {
    testImplementation(kotlin("test-junit"))
    testImplementation("org.robolectric:robolectric:4.16.1")
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
    testImplementation("com.squareup.okhttp3:okhttp-tls:4.12.0")
    testImplementation("io.ktor:ktor-client-mock:3.1.3")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}

// Fetch the SDKs used by @Config through Gradle's dependency cache before starting Robolectric.
val robolectricSdkDirectory = layout.buildDirectory.dir("robolectric-sdks")
val prepareRobolectricSdks by tasks.registering(Sync::class) {
    val sdkVersions = listOf(
        "8.0.0_r4-robolectric-r1-i7", // API 26
        "9-robolectric-4913185-2-i7", // API 28
        "13-robolectric-9030017-i7", // API 33
        "15-robolectric-13954326-i7", // API 35
    )
    sdkVersions.forEach { version ->
        // Separate configurations preserve every SDK version during conflict resolution.
        from(configurations.detachedConfiguration(dependencies.create("org.robolectric:android-all-instrumented:$version")).apply {
            isTransitive = false
        })
    }
    into(robolectricSdkDirectory)
}

tasks.withType<Test>().configureEach {
    dependsOn(prepareRobolectricSdks)
    inputs.dir(robolectricSdkDirectory).withPathSensitivity(PathSensitivity.RELATIVE)
    systemProperty("robolectric.offline", "true")
    systemProperty("robolectric.dependency.dir", robolectricSdkDirectory.get().asFile.absolutePath)
}

val electricLicense by
    tasks.registering(Sync::class) {
        dependsOn(":electric4k:unpackUpstream")
        from(project(":electric4k").layout.buildDirectory.dir("upstream")) {
            include("LICENSE", "NOTICE")
        }
        into(layout.buildDirectory.dir("generated/assets/licenses/electric4k"))
    }

tasks.named("preBuild") { dependsOn(electricLicense) }

// Bundle the shared WebRTC engine; SIP and credentials stay on the backend.
// frontend/build contains media-viewer build tools and is an input of buildMediaPreview.
// Keep generated phone assets in this module so neither bundler consumes the other output.
val phoneEngine = layout.buildDirectory.dir("generated/phone-engine")
val buildAndroidPhone by tasks.registering(Exec::class) {
    val frontend = rootDir.resolve("../frontend")
    inputs.files(fileTree(frontend.resolve("src/android-phone")), fileTree(frontend.resolve("src/lib/sip")), fileTree(frontend.resolve("src/lib/call")), frontend.resolve("vite.android-phone.config.ts"), frontend.resolve("scripts/android-phone-smoke.mjs"), frontend.resolve("package.json"), frontend.resolve("pnpm-lock.yaml"))
    outputs.dir(phoneEngine)
    workingDir(frontend)
    commandLine("pnpm", "build:android-phone")
}
val androidPhoneAssets by tasks.registering(Sync::class) {
    dependsOn(buildAndroidPhone)
    from(phoneEngine) { include("engine.js") }
    into(layout.buildDirectory.dir("generated/assets/phone"))
}
tasks.named("preBuild") { dependsOn(androidPhoneAssets) }
