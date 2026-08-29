package dev.doria.intellij.actions

import org.eclipse.lsp4j.CodeAction

class DoriaImportIntention : DoriaLspCodeActionIntention() {
    override fun getText(): String = "Use import"

    override fun getFamilyName(): String = "Doria imports"

    override val chooserTitle: String = "Choose the declaration to import"
    override val commandName: String = "Use Doria import"

    override fun accepts(action: CodeAction): Boolean = action.title.startsWith("Use import for ")
}
