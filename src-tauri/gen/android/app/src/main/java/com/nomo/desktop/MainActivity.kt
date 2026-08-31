package com.nomo.desktop

import android.os.Bundle
import android.content.Intent
import androidx.activity.enableEdgeToEdge
import java.util.UUID

class MainActivity : TauriActivity() {
  private var shareFingerprint: String? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    normalizeSharedText(intent, savedInstanceState?.getString("nomo.sharedTextFingerprint"))
    super.onCreate(savedInstanceState)
  }

  override fun onNewIntent(intent: Intent) {
    normalizeSharedText(intent)
    super.onNewIntent(intent)
    setIntent(intent)
  }

  override fun onSaveInstanceState(outState: Bundle) {
    shareFingerprint?.let { outState.putString("nomo.sharedTextFingerprint", it) }
    super.onSaveInstanceState(outState)
  }

  private fun normalizeSharedText(intent: Intent?, restoredFingerprint: String? = null) {
    shareFingerprint = null
    if (intent?.action != Intent.ACTION_SEND || intent.type != "text/plain" ||
      intent.hasExtra(Intent.EXTRA_STREAM)) return
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty()
    val normalized = prepareSharedText(text, UUID.randomUUID().toString(), restoredFingerprint)
    intent.putExtra(Intent.EXTRA_TEXT, normalized)
    shareFingerprint = sharedTextFingerprint(normalized)
  }
}
