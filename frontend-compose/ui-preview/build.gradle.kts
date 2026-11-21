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
    implementation("org.jetbrains.compose.material3:material3:1.9.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.0")
    testImplementation("androidx.activity:activity-compose:1.10.1")
    testImplementation("androidx.compose.ui:ui-test-junit4:1.9.0")
    testImplementation("org.robolectric:robolectric:4.16.1")
}
