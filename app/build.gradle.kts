plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "com.example.folialens"
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "com.example.folialens"
        minSdk = 31
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            optimization {
                enable = true
                packageScope = setOf("androidx.**", "kotlin.**", "kotlinx.**")
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true
    }

    sourceSets {
        getByName("main") {
            java.srcDir("build/generated/source/uniffi/main/java")
        }
    }
}

val com.android.build.api.dsl.ApplicationExtension.ndkDirectory: java.io.File?
    get() = project.extensions.findByType(com.android.build.gradle.BaseExtension::class.java)?.ndkDirectory

val isWindows = System.getProperty("os.name").lowercase().contains("windows")

val buildRustNative = tasks.register<Exec>("buildRustNative") {
    workingDir = file("../rust-core")
    android.ndkDirectory?.let { ndkDir ->
        environment("ANDROID_NDK_HOME", ndkDir.absolutePath)
    }
    if (isWindows) {
        commandLine(
            "cmd", "/c", "cargo", "ndk",
            "-t", "arm64-v8a",
            "-t", "x86_64",
            "-o", "../app/src/main/jniLibs",
            "build", "--release"
        )
    } else {
        commandLine(
            "cargo", "ndk",
            "-t", "arm64-v8a",
            "-t", "x86_64",
            "-o", "../app/src/main/jniLibs",
            "build", "--release"
        )
    }
}

val generateUniffiBindings = tasks.register<Exec>("generateUniffiBindings") {
    dependsOn(buildRustNative)
    workingDir = file("../rust-core")
    val outDir = file("build/generated/source/uniffi/main/java")
    doFirst {
        outDir.mkdirs()
    }
    if (isWindows) {
        commandLine(
            "cmd", "/c", "cargo", "run", "--bin", "uniffi-bindgen", "generate",
            "--library", "../app/src/main/jniLibs/arm64-v8a/libatlas_core.so",
            "--config", "uniffi.toml",
            "--language", "kotlin",
            "--out-dir", "../app/build/generated/source/uniffi/main/java"
        )
    } else {
        commandLine(
            "cargo", "run", "--bin", "uniffi-bindgen", "generate",
            "--library", "../app/src/main/jniLibs/arm64-v8a/libatlas_core.so",
            "--config", "uniffi.toml",
            "--language", "kotlin",
            "--out-dir", "../app/build/generated/source/uniffi/main/java"
        )
    }
}

tasks.named("preBuild").configure {
    dependsOn(generateUniffiBindings)
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    dependsOn(generateUniffiBindings)
    source(file("build/generated/source/uniffi/main/java"))
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)

    // JNA (for UniFFI Rust bindings)
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // LiteRT (Edge-AI Inference on CPU, GPU, NPU)
    implementation("com.google.ai.edge.litert:litert:1.0.1")
    implementation("com.google.ai.edge.litert:litert-gpu:1.0.1")
    implementation("com.google.ai.edge.litert:litert-support:1.0.1") {
        exclude(group = "com.google.ai.edge.litert", module = "litert-support-api")
    }

    // CameraX 1.4.0
    val cameraxVersion = "1.4.0"
    implementation("androidx.camera:camera-core:$cameraxVersion")
    implementation("androidx.camera:camera-camera2:$cameraxVersion")
    implementation("androidx.camera:camera-lifecycle:$cameraxVersion")
    implementation("androidx.camera:camera-view:$cameraxVersion")

    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}
