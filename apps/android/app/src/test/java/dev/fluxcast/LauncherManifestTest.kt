package dev.fluxcast

import dev.fluxcast.core.uniffiEncodeMasterKeyB64
import dev.fluxcast.core.uniffiGenerateMasterKey
import dev.fluxcast.core.uniffiHlcShouldApplyRemote
import dev.fluxcast.core.uniffiOpenEntity
import dev.fluxcast.core.uniffiParsePairingQr
import dev.fluxcast.core.uniffiSealEntity
import dev.fluxcast.data.ContentTools
import dev.fluxcast.share.ShareReceiverActivity
import dev.fluxcast.tile.SendClipboardTileService
import dev.fluxcast.widget.FluxCastWidgetReceiver
import java.util.Properties
import java.util.UUID
import javax.xml.parsers.DocumentBuilderFactory
import javax.xml.xpath.XPathFactory
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LauncherManifestTest {
    private fun loadMergedManifest() =
        Properties().let { config ->
            requireNotNull(javaClass.getResourceAsStream("/com/android/tools/test_config.properties"))
                .use { config.load(it) }
            DocumentBuilderFactory.newInstance()
                .apply { isNamespaceAware = true }
                .newDocumentBuilder()
                .parse(java.io.File(config.getProperty("android_merged_manifest")))
        }

    @Test
    fun launcherShareSheetTileAndGlanceWidgetAreDeclaredInMergedManifest() {
        val manifest = loadMergedManifest()
        val query = XPathFactory.newInstance().newXPath()

        val launcher = "/manifest/application/activity[@*[local-name()='exported']='true']" +
            "[intent-filter[action/@*[local-name()='name']='android.intent.action.MAIN']" +
            "[category/@*[local-name()='name']='android.intent.category.LAUNCHER']]"
        assertEquals(MainActivity::class.java.name, query.evaluate("$launcher/@*[local-name()='name']", manifest))

        val shareSheet = "/manifest/application/activity[@*[local-name()='exported']='true']" +
            "[intent-filter[action/@*[local-name()='name']='android.intent.action.SEND']]"
        assertEquals(
            ShareReceiverActivity::class.java.name,
            query.evaluate("$shareSheet/@*[local-name()='name']", manifest),
        )

        val qsTile = "/manifest/application/service[@*[local-name()='permission']='android.permission.BIND_QUICK_SETTINGS_TILE']"
        assertEquals(
            SendClipboardTileService::class.java.name,
            query.evaluate("$qsTile/@*[local-name()='name']", manifest),
        )

        val glanceWidget = "/manifest/application/receiver[intent-filter[action/@*[local-name()='name']='android.appwidget.action.APPWIDGET_UPDATE']]"
        assertEquals(
            FluxCastWidgetReceiver::class.java.name,
            query.evaluate("$glanceWidget/@*[local-name()='name']", manifest),
        )
    }

    @Test
    fun uniffiRustCoreEncryptsDecryptsValidatesQrAndResolvesHlcInKotlin() {
        val key = uniffiGenerateMasterKey()
        assertEquals(32, key.size)

        val entityId = UUID.randomUUID().toString()
        val hlc = ContentTools.nextHlc(1_700_000_000_000L, "android")
        val payload = """{"content_type":"url","content":"https://fedoraproject.org/kde/"}""".encodeToByteArray()

        val sealed = uniffiSealEntity(key, entityId, 1, hlc, payload)
        assertEquals(24, sealed.nonce.size)
        assertEquals(32, sealed.dedupHmac.size)

        val opened = uniffiOpenEntity(key, entityId, 1, hlc, sealed.nonce, sealed.ciphertext)
        assertArrayEquals(payload, opened)

        val keyB64 = uniffiEncodeMasterKeyB64(key)
        val qrJson = """{"version":1,"server_url":"http://127.0.0.1:8787","pairing_token":"pair_ok","master_key_b64":"$keyB64","expires_at":2000}"""
        val session = uniffiParsePairingQr(qrJson, 1950)
        assertEquals("http://127.0.0.1:8787", session.serverUrl)
        assertArrayEquals(key, session.masterKey)

        assertTrue(uniffiHlcShouldApplyRemote("001700000000100-0000-android", "001700000000200-0000-fedora"))
        assertEquals("json", ContentTools.classify("""{"ok":true}"""))
        assertTrue(ContentTools.expandTemplate("svc={{clipboard}} d={{date}}", "fluxcast", 0L).contains("svc=fluxcast d=1970-01-01"))

        // Alterar el AAD (por ejemplo HLC manipulado) debe rechazar el descifrado AEAD en UniFFI.
        val tamperedResult = runCatching {
            uniffiOpenEntity(key, entityId, 1, "999999999999999-0000-attacker", sealed.nonce, sealed.ciphertext)
        }
        assertTrue(tamperedResult.isFailure)
    }
}
