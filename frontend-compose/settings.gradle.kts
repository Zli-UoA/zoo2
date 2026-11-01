pluginManagement {
    repositories {
        google()
        gradlePluginPortal()
        mavenCentral()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
        exclusiveContent {
            forRepository {
                ivy {
                    name = "Electric4kSource"
                    url = uri("https://github.com/Kai-Z-JP/Electric4k/archive/")
                    patternLayout { artifact("[revision].tar.gz") }
                    metadataSources { artifact() }
                }
            }
            filter { includeModule("io.github.kai-z-jp", "electric4k-source") }
        }
    }
}
rootProject.name = "zoo-compose"
include(":electric4k")
