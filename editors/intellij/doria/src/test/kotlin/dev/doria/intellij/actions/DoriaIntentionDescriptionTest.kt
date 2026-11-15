package dev.doria.intellij.actions

import junit.framework.TestCase

/**
 * The platform reads `intentionDescriptions/<IntentionClass>/description.html`
 * for every registered intention and logs a SEVERE plugin error when one is
 * missing. Every `<intentionAction>` in the descriptors must ship one.
 */
class DoriaIntentionDescriptionTest : TestCase() {
    fun testEveryRegisteredIntentionShipsADescription() {
        val intentions = listOf("/META-INF/plugin.xml", "/META-INF/doria-lsp.xml")
            .mapNotNull { javaClass.getResource(it)?.readText() }
            .flatMap { descriptor ->
                INTENTION_CLASS.findAll(descriptor).map { it.groupValues[1] }.toList()
            }

        assertFalse("no intention registrations were found", intentions.isEmpty())
        for (className in intentions) {
            val shortName = className.substringAfterLast('.')
            val description = javaClass.getResource("/intentionDescriptions/$shortName/description.html")
            assertNotNull("$className has no intentionDescriptions/$shortName/description.html", description)
            assertTrue(
                "$className has an empty description",
                requireNotNull(description).readText().isNotBlank(),
            )
        }
    }

    private companion object {
        val INTENTION_CLASS = Regex(
            """<intentionAction>\s*(?:<language>[^<]*</language>\s*)?<className>([^<]+)</className>""",
        )
    }
}
