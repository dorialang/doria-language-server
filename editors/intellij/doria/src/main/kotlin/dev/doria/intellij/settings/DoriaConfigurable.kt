package dev.doria.intellij.settings

import com.intellij.openapi.options.SearchableConfigurable
import com.intellij.openapi.project.ProjectManager
import com.intellij.platform.lsp.api.LspServerManager
import com.intellij.ui.components.JBLabel
import dev.doria.intellij.lsp.DoriaLspServerSupportProvider
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.Insets
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.JTextField

class DoriaConfigurable : SearchableConfigurable {
    private var languageServerPathField: JTextField? = null
    private var batonPathField: JTextField? = null

    override fun getId(): String = "dev.doria.intellij.settings"

    override fun getDisplayName(): String = "Doria"

    override fun createComponent(): JComponent {
        val settings = DoriaSettings.getInstance().state
        val panel = JPanel(GridBagLayout())
        val constraints = GridBagConstraints().apply {
            gridx = 0
            gridy = 0
            anchor = GridBagConstraints.WEST
            insets = Insets(0, 0, 8, 8)
        }

        panel.add(JBLabel("Language server path:"), constraints)

        languageServerPathField = JTextField(settings.languageServerPath, 40)
        constraints.gridx = 1
        constraints.weightx = 1.0
        constraints.fill = GridBagConstraints.HORIZONTAL
        panel.add(languageServerPathField, constraints)

        constraints.gridx = 1
        constraints.gridy = 1
        constraints.insets = Insets(0, 0, 0, 0)
        constraints.fill = GridBagConstraints.NONE
        constraints.weightx = 0.0
        panel.add(
            JBLabel("Leave empty to use the version-matched language server bundled with the plugin."),
            constraints,
        )

        constraints.gridx = 0
        constraints.gridy = 2
        constraints.insets = Insets(12, 0, 8, 8)
        constraints.fill = GridBagConstraints.NONE
        panel.add(JBLabel("Baton path:"), constraints)

        batonPathField = JTextField(settings.batonPath, 40)
        constraints.gridx = 1
        constraints.weightx = 1.0
        constraints.fill = GridBagConstraints.HORIZONTAL
        panel.add(batonPathField, constraints)

        constraints.gridy = 3
        constraints.insets = Insets(0, 0, 0, 0)
        constraints.fill = GridBagConstraints.NONE
        constraints.weightx = 0.0
        panel.add(
            JBLabel("Leave empty to use DORIA_BATON_PATH, the installed toolchain, or Baton on PATH."),
            constraints,
        )

        return panel
    }

    override fun isModified(): Boolean {
        val settings = DoriaSettings.getInstance().state
        return languageServerPathField?.text.orEmpty() != settings.languageServerPath ||
            batonPathField?.text.orEmpty() != settings.batonPath
    }

    override fun apply() {
        val settings = DoriaSettings.getInstance().state
        val before = settings.copy()
        settings.languageServerPath = languageServerPathField?.text.orEmpty().trim()
        settings.batonPath = batonPathField?.text.orEmpty().trim()
        if (requiresServerRestart(before, settings)) {
            restartLanguageServers()
        }
    }

    /** Apply new paths now, instead of at the next IDE start. */
    private fun restartLanguageServers() {
        for (project in ProjectManager.getInstance().openProjects) {
            if (!project.isDisposed) {
                LspServerManager.getInstance(project)
                    .stopAndRestartIfNeeded(DoriaLspServerSupportProvider::class.java)
            }
        }
    }

    companion object {
        /**
         * The language server receives both paths only when it starts, so a
         * change to either takes effect only through a restart.
         */
        internal fun requiresServerRestart(before: DoriaSettings.State, after: DoriaSettings.State): Boolean =
            before.languageServerPath != after.languageServerPath || before.batonPath != after.batonPath
    }

    override fun reset() {
        languageServerPathField?.text = DoriaSettings.getInstance().state.languageServerPath
        batonPathField?.text = DoriaSettings.getInstance().state.batonPath
    }

    override fun disposeUIResources() {
        languageServerPathField = null
        batonPathField = null
    }
}
