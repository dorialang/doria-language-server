package dev.doria.intellij.actions

import org.eclipse.lsp4j.CodeAction

class DoriaGenerateFunctionIntention : DoriaLspCodeActionIntention() {
    override fun getText(): String = "Generate missing function"

    override fun getFamilyName(): String = "Doria code generation"

    override val chooserTitle: String = "Choose the function to generate"
    override val commandName: String = "Generate Doria function"

    override fun accepts(action: CodeAction): Boolean = action.title.startsWith("Generate function `")
}
