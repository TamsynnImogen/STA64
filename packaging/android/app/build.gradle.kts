plugins {
    id("com.android.application")
}

android {
    namespace = "org.sta64"
    compileSdk = 36
    buildToolsVersion = "36.0.0"
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "org.sta64"
        minSdk = 26
        targetSdk = 36
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    sourceSets.getByName("main").jniLibs.srcDir("build/generated/jniLibs")

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            // No release signing config: the APK is deliberately unsigned.
        }
    }
}
