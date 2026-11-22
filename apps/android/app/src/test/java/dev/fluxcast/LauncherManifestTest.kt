package dev.fluxcast

import java.util.Properties
import javax.xml.parsers.DocumentBuilderFactory
import javax.xml.xpath.XPathFactory
import org.junit.Assert.assertEquals
import org.junit.Test

class LauncherManifestTest {
    @Test
    fun launcherResolvesToExportedMainActivity() {
        val config = Properties()
        requireNotNull(javaClass.getResourceAsStream("/com/android/tools/test_config.properties"))
            .use { config.load(it) }
        val manifest = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }.newDocumentBuilder()
            .parse(java.io.File(config.getProperty("android_merged_manifest")))
        val query = XPathFactory.newInstance().newXPath()
        val launcher = "/manifest/application/activity[@*[local-name()='exported']='true']" +
            "[intent-filter[action/@*[local-name()='name']='android.intent.action.MAIN']" +
            "[category/@*[local-name()='name']='android.intent.category.LAUNCHER']]"
        assertEquals(MainActivity::class.java.name, query.evaluate("$launcher/@*[local-name()='name']", manifest))
    }
}
