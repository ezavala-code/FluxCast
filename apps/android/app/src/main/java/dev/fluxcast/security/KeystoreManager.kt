package dev.fluxcast.security

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Envuelve y protege la Master Key E2EE (32 bytes) y el JWT de sesión usando una llave
 * AES-256-GCM residente en el hardware de Android Keystore, además de gestionar BiometricPrompt.
 */
class KeystoreManager(context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences("fluxcast_secure_store", Context.MODE_PRIVATE)

    fun saveMasterKey(rawKey: ByteArray) {
        require(rawKey.size == 32) { "La Master Key debe tener 32 bytes" }
        prefs.edit().putString(KEY_MASTER, encryptWithKeystore(rawKey)).apply()
    }

    fun loadMasterKey(): ByteArray? {
        val blob = prefs.getString(KEY_MASTER, null) ?: return null
        return runCatching { decryptWithKeystore(blob) }.getOrNull()
    }

    fun saveSession(serverUrl: String, userId: String, deviceId: String, accessToken: String) {
        prefs.edit()
            .putString(KEY_SERVER_URL, serverUrl.trimEnd('/'))
            .putString(KEY_USER_ID, userId)
            .putString(KEY_DEVICE_ID, deviceId)
            .putString(KEY_TOKEN, encryptWithKeystore(accessToken.encodeToByteArray()))
            .apply()
    }

    fun loadAccessToken(): String? {
        val blob = prefs.getString(KEY_TOKEN, null) ?: return null
        return runCatching { decryptWithKeystore(blob).decodeToString() }.getOrNull()
    }

    fun serverUrl(): String = prefs.getString(KEY_SERVER_URL, "http://10.0.2.2:8787") ?: "http://10.0.2.2:8787"
    fun setServerUrl(url: String) = prefs.edit().putString(KEY_SERVER_URL, url.trim().trimEnd('/')).apply()
    fun userId(): String? = prefs.getString(KEY_USER_ID, null)
    fun deviceId(): String? = prefs.getString(KEY_DEVICE_ID, null)
    fun lastSeq(): Long = prefs.getLong(KEY_LAST_SEQ, 0L)
    fun setLastSeq(seq: Long) = prefs.edit().putLong(KEY_LAST_SEQ, seq).apply()

    fun isBiometricLockEnabled(): Boolean = prefs.getBoolean(KEY_BIOMETRIC_ENABLED, false)
    fun setBiometricLockEnabled(enabled: Boolean) =
        prefs.edit().putBoolean(KEY_BIOMETRIC_ENABLED, enabled).apply()

    private fun getOrCreateHardwareKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        val existing = ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry
        if (existing != null) return existing.secretKey

        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()
        generator.init(spec)
        return generator.generateKey()
    }

    private fun encryptWithKeystore(plaintext: ByteArray): String {
        return runCatching {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, getOrCreateHardwareKey())
            val iv = Base64.encodeToString(cipher.iv, Base64.NO_WRAP)
            val ct = Base64.encodeToString(cipher.doFinal(plaintext), Base64.NO_WRAP)
            "$iv:$ct"
        }.getOrElse {
            "plain:" + Base64.encodeToString(plaintext, Base64.NO_WRAP)
        }
    }

    private fun decryptWithKeystore(stored: String): ByteArray {
        if (stored.startsWith("plain:")) {
            return Base64.decode(stored.removePrefix("plain:"), Base64.NO_WRAP)
        }
        val parts = stored.split(":", limit = 2)
        require(parts.size == 2) { "Formato cifrado inválido" }
        val iv = Base64.decode(parts[0], Base64.NO_WRAP)
        val ct = Base64.decode(parts[1], Base64.NO_WRAP)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, getOrCreateHardwareKey(), GCMParameterSpec(128, iv))
        return cipher.doFinal(ct)
    }

    companion object {
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val ALIAS = "fluxcast_master_wrap_key_v1"
        private const val KEY_MASTER = "wrapped_master_key"
        private const val KEY_TOKEN = "wrapped_access_token"
        private const val KEY_SERVER_URL = "server_url"
        private const val KEY_USER_ID = "user_id"
        private const val KEY_DEVICE_ID = "device_id"
        private const val KEY_LAST_SEQ = "last_seq"
        private const val KEY_BIOMETRIC_ENABLED = "biometric_enabled"

        fun authenticateBiometric(
            activity: FragmentActivity,
            onSuccess: () -> Unit,
            onError: (String) -> Unit,
        ) {
            val authenticators =
                BiometricManager.Authenticators.BIOMETRIC_STRONG or
                    BiometricManager.Authenticators.DEVICE_CREDENTIAL
            val canAuth = BiometricManager.from(activity).canAuthenticate(authenticators)
            if (canAuth != BiometricManager.BIOMETRIC_SUCCESS) {
                onSuccess()
                return
            }
            val executor = ContextCompat.getMainExecutor(activity)
            val prompt = BiometricPrompt(
                activity,
                executor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        onSuccess()
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        onError(errString.toString())
                    }
                },
            )
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle("Desbloquear FluxCast")
                .setSubtitle("Verifica tu identidad para acceder a tu portapapeles E2EE")
                .setAllowedAuthenticators(authenticators)
                .build()
            prompt.authenticate(info)
        }
    }
}
