package com.nomo.desktop

import java.net.URLEncoder
import java.security.MessageDigest

/** Keep shared text (including URLs) as text before tao tries to parse it as a URL. */
internal fun sharedTextDataUrl(text: String, deliveryId: String): String =
  "data:text/plain," + URLEncoder.encode(text, "UTF-8").replace("+", "%20") +
    "#nomo-share=" + deliveryId

internal fun sharedTextFingerprint(text: String): String =
  MessageDigest.getInstance("SHA-256").digest(text.toByteArray(Charsets.UTF_8))
    .joinToString("") { "%02x".format(it.toInt() and 0xff) }

/** Only our saved Activity state may identify a replay; incoming Intent extras are untrusted. */
internal fun prepareSharedText(text: String, deliveryId: String, restoredFingerprint: String?): String =
  if (restoredFingerprint != null && sharedTextFingerprint(text) == restoredFingerprint) text
  else sharedTextDataUrl(text, deliveryId)
