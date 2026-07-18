package dev.doria.intellij.formatting

import com.intellij.application.options.CodeStyleAbstractConfigurable
import com.intellij.application.options.CodeStyleAbstractPanel
import com.intellij.application.options.TabbedLanguageCodeStylePanel
import com.intellij.lang.Language
import com.intellij.psi.codeStyle.CodeStyleConfigurable
import com.intellij.psi.codeStyle.CodeStyleSettings
import com.intellij.psi.codeStyle.CodeStyleSettingsCustomizable
import com.intellij.psi.codeStyle.CodeStyleSettingsProvider
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.psi.codeStyle.CustomCodeStyleSettings
import com.intellij.psi.codeStyle.LanguageCodeStyleSettingsProvider
import dev.doria.intellij.DoriaLanguage

class DoriaCodeStyleSettings(settings: CodeStyleSettings) :
    CustomCodeStyleSettings("DoriaCodeStyleSettings", settings)

class DoriaCodeStyleSettingsProvider : CodeStyleSettingsProvider() {
    override fun createCustomSettings(settings: CodeStyleSettings): CustomCodeStyleSettings =
        DoriaCodeStyleSettings(settings)

    override fun getConfigurableDisplayName(): String = "Doria"

    override fun createConfigurable(
        settings: CodeStyleSettings,
        modelSettings: CodeStyleSettings,
    ): CodeStyleConfigurable = object : CodeStyleAbstractConfigurable(
        settings,
        modelSettings,
        configurableDisplayName,
    ) {
        override fun createPanel(settings: CodeStyleSettings): CodeStyleAbstractPanel =
            DoriaCodeStyleMainPanel(currentSettings, settings)
    }
}

private class DoriaCodeStyleMainPanel(
    currentSettings: CodeStyleSettings,
    settings: CodeStyleSettings,
) : TabbedLanguageCodeStylePanel(DoriaLanguage, currentSettings, settings)

class DoriaLanguageCodeStyleSettingsProvider : LanguageCodeStyleSettingsProvider() {
    override fun getLanguage(): Language = DoriaLanguage

    override fun customizeSettings(
        consumer: CodeStyleSettingsCustomizable,
        settingsType: SettingsType,
    ) {
        if (settingsType == SettingsType.WRAPPING_AND_BRACES_SETTINGS) {
            consumer.showStandardOptions(
                "BRACE_STYLE",
                "ELSE_ON_NEW_LINE",
                "CATCH_ON_NEW_LINE",
                "FINALLY_ON_NEW_LINE",
            )
            consumer.renameStandardOption("BRACE_STYLE", "Brace placement")
        }
    }

    override fun getCodeSample(settingsType: SettingsType): String = CODE_SAMPLE

    @Suppress("OVERRIDE_DEPRECATION")
    override fun getDefaultCommonSettings(): CommonCodeStyleSettings =
        CommonCodeStyleSettings(DoriaLanguage).apply {
            initIndentOptions()
            indentOptions?.INDENT_SIZE = 4
            indentOptions?.CONTINUATION_INDENT_SIZE = 4
            BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        }

    companion object {
        private val CODE_SAMPLE = """
            class Greeter
            {
                function greet(string ${'$'}name): void
                {
                    if (${'$'}name == "Doria")
                    {
                        echo "Hello, {${'$'}name}!";
                    }
                    else
                    {
                        echo "Hello!";
                    }
                }
            }
        """.trimIndent()
    }
}
