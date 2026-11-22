package dev.fluxcast

import android.os.Build
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import dev.fluxcast.security.KeystoreManager
import dev.fluxcast.ui.FluxCastRootScreen

class MainActivity : FragmentActivity() {
    private var unlocked by mutableStateOf(true)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = application as FluxCastApp
        if (app.keystore.isBiometricLockEnabled()) {
            unlocked = false
            triggerBiometricUnlock()
        }

        setContent {
            val darkTheme = isSystemInDarkTheme()
            val colorScheme = when {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
                    if (darkTheme) dynamicDarkColorScheme(this) else dynamicLightColorScheme(this)
                }
                darkTheme -> darkColorScheme()
                else -> lightColorScheme()
            }
            MaterialTheme(colorScheme = colorScheme) {
                Surface(modifier = Modifier.fillMaxSize()) {
                    if (unlocked) {
                        FluxCastRootScreen(repository = app.repository)
                    } else {
                        Column(
                            modifier = Modifier
                                .fillMaxSize()
                                .padding(24.dp),
                            verticalArrangement = Arrangement.Center,
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Text("Bóveda E2EE bloqueada", style = MaterialTheme.typography.headlineSmall)
                            Button(
                                onClick = { triggerBiometricUnlock() },
                                modifier = Modifier.padding(top = 12.dp),
                            ) {
                                Text("Desbloquear con Biometría")
                            }
                        }
                    }
                }
            }
        }
    }

    private fun triggerBiometricUnlock() {
        KeystoreManager.authenticateBiometric(
            activity = this,
            onSuccess = { unlocked = true },
            onError = { unlocked = false },
        )
    }
}
