plugins {
    id("com.android.library")
    kotlin("android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "zoo.android.uipreview"
    compileSdk = 35
    defaultConfig { minSdk = 26 }
    sourceSets["main"].java.srcDir("../androidApp/src/main/kotlin/zoo/android/ui")
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    testOptions.unitTests.isIncludeAndroidResources = true
}

kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }

dependencies {
    implementation(project(":"))
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("org.jetbrains.compose.material3:material3:1.9.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.0")
    testImplementation("androidx.activity:activity-compose:1.10.1")
    testImplementation("androidx.compose.ui:ui-test-junit4:1.9.0")
    testImplementation("org.robolectric:robolectric:4.16.1")
}

// Use Gradle's cached API 35 SDK for the Compose tests.
val robolectricSdkDirectory = layout.buildDirectory.dir("robolectric-sdks")
val prepareRobolectricSdk by tasks.registering(Sync::class) {
    from(configurations.detachedConfiguration(
        dependencies.create("org.robolectric:android-all-instrumented:15-robolectric-13954326-i7"),
    ).apply { isTransitive = false })
    into(robolectricSdkDirectory)
}

tasks.withType<Test>().configureEach {
    dependsOn(prepareRobolectricSdk)
    inputs.dir(robolectricSdkDirectory).withPathSensitivity(PathSensitivity.RELATIVE)
    systemProperty("robolectric.offline", "true")
    systemProperty("robolectric.dependency.dir", robolectricSdkDirectory.get().asFile.absolutePath)
}
