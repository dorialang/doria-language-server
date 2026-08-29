package dev.doria.intellij.actions

import org.eclipse.lsp4j.CodeAction

class DoriaGenerateMethodIntention : DoriaLspCodeActionIntention() {
    override fun getText(): String = "Generate missing method"

    override fun getFamilyName(): String = "Doria code generation"

    override val chooserTitle: String = "Choose the method to generate"
    override val commandName: String = "Generate Doria method"

    override fun accepts(action: CodeAction): Boolean = action.title.startsWith("Generate method `")
}
