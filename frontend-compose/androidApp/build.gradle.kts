plugins {
    id("com.android.application")
    kotlin("android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "works.zli.zoo"
    compileSdk = 35
    defaultConfig {
        applicationId = "works.zli.zoo"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
}

dependencies {
    implementation(project(":"))
    implementation("com.caverock:androidsvg-aar:1.4")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("org.jetbrains.compose.material3:material3:1.9.0")
    implementation("io.ktor:ktor-client-okhttp:3.1.3")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")
}

val translations by
    tasks.registering(Sync::class) {
        from("../../frontend/src/lib/i18n/locales")
        into(layout.buildDirectory.dir("generated/assets/locales"))
    }

android.sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/assets"))

tasks.named("preBuild") { dependsOn(translations) }

dependencies {
    testImplementation(kotlin("test-junit"))
    testImplementation("io.ktor:ktor-client-mock:3.1.3")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
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

val launcherIcon by
    tasks.registering(Sync::class) {
        from("../../frontend/public/icons/icon-512.png") { rename { "zoo_launcher.png" } }
        into(layout.buildDirectory.dir("generated/launcherRes/drawable-nodpi"))
    }

android.sourceSets["main"].res.srcDir(layout.buildDirectory.dir("generated/launcherRes"))

tasks.named("preBuild") { dependsOn(launcherIcon) }
