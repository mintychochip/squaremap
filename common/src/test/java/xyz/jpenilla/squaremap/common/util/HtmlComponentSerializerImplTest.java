package xyz.jpenilla.squaremap.common.util;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import net.kyori.adventure.text.Component;
import net.kyori.adventure.text.flattener.ComponentFlattener;
import net.kyori.adventure.text.format.NamedTextColor;
import net.kyori.adventure.text.format.TextDecoration;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.api.HtmlComponentSerializer;

class HtmlComponentSerializerImplTest {
    private final HtmlComponentSerializer serializer =
        new HtmlComponentSerializerImpl(ComponentFlattener.basic());

    @Test
    void escapesHtmlMarkupInComponentText() {
        final String html = this.serializer.serialize(Component.text("<b>admin</b>"));
        assertFalse(html.contains("<b>"), html);
        assertTrue(html.contains("&lt;b&gt;admin&lt;/b&gt;"), html);
    }

    @Test
    void escapesImageTagInComponentText() {
        final String html = this.serializer.serialize(Component.text("<img src=x onerror=alert(1)>"));
        assertFalse(html.toLowerCase().contains("<img"), html);
        assertTrue(html.contains("&lt;img"), html);
        assertTrue(html.contains("onerror"), html);
        assertTrue(html.contains("alert(1)"), html);
    }

    @Test
    void preservesColorSpansAroundEscapedText() {
        final String html = this.serializer.serialize(
            Component.text("<b>x</b>").color(NamedTextColor.RED)
        );
        assertTrue(html.contains("color:#ff5555") || html.contains("color:#FF5555"), html);
        assertTrue(html.contains("&lt;b&gt;x&lt;/b&gt;"), html);
        assertFalse(html.contains("<b>"), html);
    }

    @Test
    void stillRendersBoldDecorationAsSpan() {
        final String html = this.serializer.serialize(
            Component.text("hello").decorate(TextDecoration.BOLD)
        );
        assertTrue(html.contains("font-weight:bold"), html);
        assertTrue(html.contains("hello"), html);
        assertEquals(html.replaceAll("<[^>]+>", ""), "hello");
    }
}
