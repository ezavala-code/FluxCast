package dev.fluxcast

import android.app.Application
import dev.fluxcast.data.LocalDatabase
import dev.fluxcast.data.SyncRepository
import dev.fluxcast.security.KeystoreManager

class FluxCastApp : Application() {
    lateinit var keystore: KeystoreManager
        private set
    lateinit var database: LocalDatabase
        private set
    lateinit var repository: SyncRepository
        private set

    override fun onCreate() {
        super.onCreate()
        keystore = KeystoreManager(this)
        database = LocalDatabase(this)
        repository = SyncRepository(this, database, keystore)
    }
}
