package zoo.android.update

import kotlin.test.*
import kotlinx.serialization.json.*

class UpdateFeedTest {
    private val commit = "a".repeat(40)

    private fun feed(vararg changes: Pair<String, JsonPrimitive>): String =
        JsonObject(
                mapOf(
                    "channel" to JsonPrimitive("main"),
                    "packageName" to JsonPrimitive("works.zli.zoo"),
                    "versionCode" to JsonPrimitive(42),
                    "versionName" to JsonPrimitive("main.42-aaaaaaa"),
                    "url" to JsonPrimitive("${UPDATE_BASE}zoo-android-main-$commit.apk"),
                    "sha256" to JsonPrimitive("b".repeat(64)),
                    "size" to JsonPrimitive(100),
                ) + changes
            )
            .toString()

    @Test
    fun acceptsVersionedMainAsset() {
        val release = UpdateRelease.parse(feed())
        assertEquals(42L, release.versionCode)
        assertEquals(100L, release.size)
    }

    @Test
    fun rejectsOtherOriginsChannelsAndPackages() {
        for (changes in
            listOf(
                "url" to
                    "http://github.com/Zli-UoA/zoo2/releases/download/android-main/zoo-android-main-$commit.apk",
                "url" to
                    "https://github.com.attacker.example/Zli-UoA/zoo2/releases/download/android-main/zoo-android-main-$commit.apk",
                "url" to
                    "https://user@github.com/Zli-UoA/zoo2/releases/download/android-main/zoo-android-main-$commit.apk",
                "url" to "${UPDATE_BASE}../other/zoo-android-main-$commit.apk",
                "url" to "${UPDATE_BASE}zoo-android-main-$commit.apk?token=x",
                "url" to "${UPDATE_BASE}latest.apk",
                "channel" to "stable",
                "packageName" to "another.app",
            )) assertFailsWith<IllegalArgumentException>(changes.toString()) {
            UpdateRelease.parse(feed(changes.first to JsonPrimitive(changes.second)))
        }
    }

    @Test
    fun rejectsInvalidDigestsVersionsAndSizes() {
        for (value in listOf(0L, -1L, Int.MAX_VALUE.toLong() + 1)) {
            assertFailsWith<IllegalArgumentException> {
                UpdateRelease.parse(feed("versionCode" to JsonPrimitive(value)))
            }
        }
        for (value in listOf(0L, -1L, MAX_APK_BYTES + 1)) {
            assertFailsWith<IllegalArgumentException> {
                UpdateRelease.parse(feed("size" to JsonPrimitive(value)))
            }
        }
        assertFailsWith<IllegalArgumentException> {
            UpdateRelease.parse(feed("sha256" to JsonPrimitive("wrong")))
        }
    }

    @Test
    fun onlyAcceptsExactPackageNewerVersionAndSameSigner() {
        requireMatchingApk(
            "works.zli.zoo",
            41,
            setOf("key"),
            "works.zli.zoo",
            42,
            setOf("key"),
            UpdateRelease.parse(feed()),
        )
        for (signers in listOf(emptySet(), setOf("other"), setOf("key", "other"))) {
            assertFailsWith<IllegalArgumentException> {
                requireMatchingApk(
                    "works.zli.zoo",
                    41,
                    setOf("key"),
                    "works.zli.zoo",
                    42,
                    signers,
                    UpdateRelease.parse(feed()),
                )
            }
        }
        assertFailsWith<IllegalArgumentException> {
            requireMatchingApk(
                "works.zli.zoo",
                41,
                emptySet(),
                "works.zli.zoo",
                42,
                emptySet(),
                UpdateRelease.parse(feed()),
            )
        }
        assertFailsWith<IllegalArgumentException> {
            requireMatchingApk(
                "works.zli.zoo",
                41,
                setOf("key"),
                "other",
                42,
                setOf("key"),
                UpdateRelease.parse(feed()),
            )
        }
        assertFailsWith<IllegalArgumentException> {
            requireMatchingApk(
                "works.zli.zoo",
                42,
                setOf("key"),
                "works.zli.zoo",
                42,
                setOf("key"),
                UpdateRelease.parse(feed()),
            )
        }
        assertFailsWith<IllegalArgumentException> {
            requireMatchingApk(
                "works.zli.zoo",
                41,
                setOf("key"),
                "works.zli.zoo",
                43,
                setOf("key"),
                UpdateRelease.parse(feed()),
            )
        }
    }
}
