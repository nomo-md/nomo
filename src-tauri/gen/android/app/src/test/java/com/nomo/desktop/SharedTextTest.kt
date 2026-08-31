package com.nomo.desktop

import java.net.URLDecoder
import org.junit.Assert.*
import org.junit.Test

class SharedTextTest {
  @Test fun preservesPlainTextAndLinksForBothLifecycleEntrypoints() {
    listOf("hello world", "中文\n第二行", "https://example.com/?a=1&b=2#标题", "100% + # ?").forEach { text ->
      val url = sharedTextDataUrl(text, "delivery-1")
      assertTrue(url.startsWith("data:text/plain,"))
      assertEquals(text, URLDecoder.decode(url.substringAfter(',').substringBefore('#'), "UTF-8"))
      assertEquals(url, sharedTextDataUrl(text, "delivery-1"))
      assertNotEquals(url, sharedTextDataUrl(text, "delivery-2"))
    }
  }

  @Test fun activityReplayIsStableButNewSharesCannotBypassTextNormalization() {
    val first = prepareSharedText("https://example.com/?secret=value", "first", null)
    assertEquals(first, prepareSharedText(first, "recreated", sharedTextFingerprint(first)))
    val nextShare = prepareSharedText(first, "new-intent", null)
    assertNotEquals(first, nextShare)
    assertEquals(first, URLDecoder.decode(nextShare.substringAfter(',').substringBefore('#'), "UTF-8"))
    assertTrue(prepareSharedText("", "empty", null).startsWith("data:text/plain,"))
  }
}
