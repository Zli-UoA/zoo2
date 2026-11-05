package zoo.android

import android.content.SharedPreferences
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import kotlin.test.*

class AndroidLoginStoreTest {
    private val connection = LoginConnection("https://zoo.example", "https://id.example", "client", "https://zoo.example/api")
    private fun login(token: String) = SavedLogin(connection, token)

    private class Fixture {
        val preferences = MemoryPreferences()
        var key: SecretKey? = null
        fun store() = AndroidLoginStore(preferences, { key }, {
            KeyGenerator.getInstance("AES").apply { init(256) }.generateKey().also { key = it }
        }, { key = null })
    }

    @Test
    fun consumedTokenIsNotRestoredAfterReopeningTheStore() {
        val fixture = Fixture()
        val store = fixture.store()
        val owner = store.beginSession()
        store.save(login("old"), owner)
        store.consume(owner)
        assertFalse(store.hasSession)
        assertNull(fixture.store().load())
        store.save(login("rotated"), owner)
        assertEquals("rotated", fixture.store().load()?.refreshToken)
    }

    @Test
    fun lateWriterCannotOverwriteOrDeleteANewerLogin() {
        val fixture = Fixture()
        val first = fixture.store()
        val oldOwner = first.beginSession()
        first.save(login("old"), oldOwner)
        first.consume(oldOwner)
        val newer = fixture.store()
        val newOwner = newer.beginSession()
        newer.save(login("new login"), newOwner)
        assertFailsWith<LoginFailure> { first.save(login("late rotation"), oldOwner) }
        assertFailsWith<LoginFailure> { first.consume(oldOwner) }
        first.clear(oldOwner)
        assertEquals("new login", newer.load()?.refreshToken)
    }

    @Test
    fun staleSnapshotCannotClaimOwnershipAndSendAUsedToken() {
        val fixture = Fixture()
        val store = fixture.store()
        val owner = store.beginSession()
        store.save(login("old"), owner)
        val snapshot = store.load()!!
        store.consume(owner)
        store.save(login("rotated"), owner)
        assertFailsWith<LoginFailure> { fixture.store().beginSession(snapshot) }
        assertEquals("rotated", store.load()?.refreshToken)
        store.consume(owner) // A rejected snapshot must not invalidate the current writer.
    }

    @Test
    fun removalCommitFailureDoesNotPermitSendingTheToken() {
        val fixture = Fixture()
        val store = fixture.store()
        val owner = store.beginSession()
        store.save(login("old"), owner)
        fixture.preferences.failCommit = true
        assertFailsWith<LoginFailure> { store.consume(owner) }
    }

    private class MemoryPreferences : SharedPreferences {
        private val values = mutableMapOf<String, String?>()
        var failCommit = false
        override fun contains(key: String?) = values.containsKey(key)
        override fun getAll(): MutableMap<String, *> = values.toMutableMap()
        override fun getString(key: String?, defValue: String?) = values[key] ?: defValue
        override fun getStringSet(key: String?, defValues: MutableSet<String>?) = defValues
        override fun getInt(key: String?, defValue: Int) = defValue
        override fun getLong(key: String?, defValue: Long) = defValue
        override fun getFloat(key: String?, defValue: Float) = defValue
        override fun getBoolean(key: String?, defValue: Boolean) = defValue
        override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
        override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
        override fun edit() = object : SharedPreferences.Editor {
            val pending = values.toMutableMap()
            override fun putString(key: String?, value: String?) = apply { pending[key!!] = value }
            override fun remove(key: String?) = apply { pending.remove(key) }
            override fun clear() = apply { pending.clear() }
            override fun commit(): Boolean {
                if (failCommit) return false
                values.clear(); values.putAll(pending)
                return true
            }
            override fun apply() { commit() }
            override fun putStringSet(key: String?, values: MutableSet<String>?) = error("Not used")
            override fun putInt(key: String?, value: Int) = error("Not used")
            override fun putLong(key: String?, value: Long) = error("Not used")
            override fun putFloat(key: String?, value: Float) = error("Not used")
            override fun putBoolean(key: String?, value: Boolean) = error("Not used")
        }
    }
}
