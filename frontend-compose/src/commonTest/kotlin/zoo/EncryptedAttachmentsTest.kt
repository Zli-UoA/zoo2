package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import io.ktor.http.content.OutgoingContent
import io.ktor.http.content.TextContent
import kotlin.test.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*

/** 暗号化した添付（#364）。暗号は XOR の偽物で、送る値と手順だけを確かめる（UniFFI は androidUnitTest）。 */
class EncryptedAttachmentsTest {
    private val plaintext = "ZOO2-E2EE-364-PLAINTEXT".encodeToByteArray()
    private val fileJson = buildJsonObject { put("v", "v2") }

    private object XorCipher : AttachmentCipher {
        override fun encrypt(plaintext: ByteArray) =
            EncryptedBytes(
                ByteArray(plaintext.size) { (plaintext[it].toInt() xor 0x5a).toByte() },
                buildJsonObject { put("v", "v2") },
            )

        override fun decrypt(file: JsonObject, ciphertext: ByteArray): ByteArray {
            assertEquals("v2", file.text("v"))
            return ByteArray(ciphertext.size) { (ciphertext[it].toInt() xor 0x5a).toByte() }
        }
    }

    private class Recorded {
        var presign: JsonObject? = null
        var uploaded: ByteArray? = null
        var uploadType: String? = null
        var uploadAuth: String? = null
    }

    private fun api(recorded: Recorded, stored: () -> ByteArray = { recorded.uploaded!! }): Pair<ZooApi, HttpClient> {
        val http =
            HttpClient(
                MockEngine { request ->
                    when {
                        request.url.encodedPath == "/api/attachments/presign" -> {
                            recorded.presign =
                                Json.parseToJsonElement((request.body as TextContent).text).jsonObject
                            respond(
                                """{"data":{"attachment_id":"att-1","upload_url":"https://s3.test/put","storage_key":"k"}}""",
                                headers = headersOf(HttpHeaders.ContentType, "application/json"),
                            )
                        }
                        request.url.host == "s3.test" && request.method == HttpMethod.Put -> {
                            val body = request.body as OutgoingContent.ByteArrayContent
                            recorded.uploaded = body.bytes()
                            recorded.uploadType = body.contentType?.toString()
                            recorded.uploadAuth = request.headers["X-Dev-User"]
                            respond("")
                        }
                        request.url.encodedPath == "/api/attachments/att-1/url" ->
                            respond(
                                """{"data":{"download_url":"https://s3.test/get"}}""",
                                headers = headersOf(HttpHeaders.ContentType, "application/json"),
                            )
                        request.url.host == "s3.test" -> respond(stored())
                        else -> respondError(HttpStatusCode.NotFound)
                    }
                }
            )
        return ZooApi(Config("https://zoo.test", "/shapes", "alice", false), TestPlatform(), http) to http
    }

    private class TestPlatform : Platform {
        override suspend fun headers() = mapOf("X-Dev-User" to "alice")

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "id"

        override fun now() = "2026-11-13T00:00:00Z"

        override fun navigate(path: String) {}

        override fun translate(key: String, args: Map<String, String>) = key

        override fun action(name: String) {}

        override fun time(timestamp: String) = timestamp

        override fun day(timestamp: String) = timestamp.take(10)
    }

    @Test
    fun uploadsOnlyTheCiphertextWithFixedMetadata() = runTest {
        val recorded = Recorded()
        val (api, http) = api(recorded)
        val info =
            uploadEncryptedAttachment(api, XorCipher, "channel-1", plaintext, "secret.txt", "text/plain")

        assertEquals(encryptedPresignBody("channel-1", plaintext.size), recorded.presign)
        assertEquals(ENCRYPTED_ATTACHMENT_FILE_NAME, recorded.presign!!.text("file_name"))
        assertEquals(ENCRYPTED_ATTACHMENT_CONTENT_TYPE, recorded.uploadType)
        // S3 には認証ヘッダーを渡さず、暗号文だけを上げる。
        assertNull(recorded.uploadAuth)
        assertFalse(recorded.uploaded!!.contentEquals(plaintext))
        assertFalse(recorded.uploaded!!.decodeToString().contains("PLAINTEXT"))

        assertEquals(
            encryptedAttachmentInfo("att-1", "secret.txt", "text/plain", plaintext.size, fileJson),
            info,
        )
        assertNull(info["width"])
        assertNull(info["sensitive"])
        http.close()
    }

    @Test
    fun downloadsAndDecryptsWithThePayloadKey() = runTest {
        val recorded = Recorded()
        val (api, http) = api(recorded)
        val info =
            uploadEncryptedAttachment(api, XorCipher, "channel-1", plaintext, "a.png", "image/png", 4, 3, true)
        assertEquals(4, info.getValue("width").jsonPrimitive.int)
        assertEquals(true, info.getValue("sensitive").jsonPrimitive.boolean)

        assertContentEquals(plaintext, fetchEncryptedAttachment(api, XorCipher, info))
        http.close()

        // 大きさが本文と違えば、平文を返さない。
        val (short, shortHttp) = api(recorded) { recorded.uploaded!!.copyOf(3) }
        assertTrue(runCatching { fetchEncryptedAttachment(short, XorCipher, info) }.isFailure)
        shortHttp.close()
    }
}
