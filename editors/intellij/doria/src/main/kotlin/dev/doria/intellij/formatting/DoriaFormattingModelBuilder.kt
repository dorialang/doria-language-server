package dev.doria.intellij.formatting

import com.intellij.formatting.Alignment
import com.intellij.formatting.Block
import com.intellij.formatting.FormattingContext
import com.intellij.formatting.FormattingModel
import com.intellij.formatting.FormattingModelBuilder
import com.intellij.formatting.FormattingModelProvider
import com.intellij.formatting.Indent
import com.intellij.formatting.Spacing
import com.intellij.formatting.Wrap
import com.intellij.formatting.WrapType
import com.intellij.lang.ASTNode
import com.intellij.psi.TokenType
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.psi.formatter.common.AbstractBlock
import dev.doria.intellij.DoriaLanguage
import dev.doria.intellij.highlighting.DoriaTokenTypes

class DoriaFormattingModelBuilder : FormattingModelBuilder {
    override fun createModel(formattingContext: FormattingContext): FormattingModel {
        val settings = formattingContext.codeStyleSettings
        val commonSettings = settings.getCommonSettings(DoriaLanguage)
        val rootBlock = DoriaFormattingBlock(formattingContext.node, commonSettings)

        return FormattingModelProvider.createFormattingModelForPsiFile(
            formattingContext.containingFile,
            rootBlock,
            settings,
        )
    }
}

private class DoriaFormattingBlock(
    private val astNode: ASTNode,
    private val commonSettings: CommonCodeStyleSettings,
    private val indentLevel: Int = 0,
) : AbstractBlock(
    astNode,
    Wrap.createWrap(WrapType.NONE, false),
    Alignment.createAlignment(),
) {
    override fun buildChildren(): List<Block> {
        val blocks = mutableListOf<Block>()
        var braceDepth = 0
        var child = astNode.firstChildNode
        while (child != null) {
            if (child.elementType != TokenType.WHITE_SPACE && child.textLength > 0) {
                if (child.isStructuralBrace("}")) {
                    braceDepth = (braceDepth - 1).coerceAtLeast(0)
                }
                blocks += DoriaFormattingBlock(child, commonSettings, braceDepth)
                if (child.isStructuralBrace("{")) {
                    braceDepth++
                }
            }
            child = child.treeNext
        }
        return blocks
    }

    override fun getIndent(): Indent = Indent.getSpaceIndent(
        indentLevel * (commonSettings.indentOptions?.INDENT_SIZE ?: DEFAULT_INDENT_SIZE),
    )

    override fun getSpacing(child1: Block?, child2: Block): Spacing? {
        val left = (child1 as? DoriaFormattingBlock)?.astNode ?: return null
        val right = (child2 as? DoriaFormattingBlock)?.astNode ?: return null

        if (right.isStructuralBrace("{") && left.elementType !in COMMENT_TOKENS) {
            return openingBraceSpacing(commonSettings.BRACE_STYLE)
        }

        if (left.text == "}" && right.text in CONTROL_FLOW_CONTINUATIONS) {
            return continuationSpacing(right.text)
        }

        return null
    }

    override fun isLeaf(): Boolean = astNode.firstChildNode == null

    private fun continuationSpacing(keyword: String): Spacing {
        val onNewLine = when (keyword) {
            "else" -> commonSettings.ELSE_ON_NEW_LINE
            "catch" -> commonSettings.CATCH_ON_NEW_LINE
            "finally" -> commonSettings.FINALLY_ON_NEW_LINE
            else -> false
        }
        return if (onNewLine) {
            lineBreak()
        } else {
            singleSpace()
        }
    }

    companion object {
        private const val DEFAULT_INDENT_SIZE = 4
        private val COMMENT_TOKENS = setOf(
            DoriaTokenTypes.COMMENT,
            DoriaTokenTypes.DOC_COMMENT,
            DoriaTokenTypes.DOC_COMMENT_TAG,
        )
        private val CONTROL_FLOW_CONTINUATIONS = setOf("else", "catch", "finally")

        private fun openingBraceSpacing(braceStyle: Int): Spacing =
            if (braceStyle == CommonCodeStyleSettings.END_OF_LINE ||
                braceStyle == CommonCodeStyleSettings.NEXT_LINE_IF_WRAPPED
            ) {
                singleSpace()
            } else {
                lineBreak()
            }

        private fun singleSpace(): Spacing = Spacing.createSpacing(1, 1, 0, false, 0)

        private fun lineBreak(): Spacing = Spacing.createSpacing(0, 0, 1, false, 0)
    }
}

private fun ASTNode.isStructuralBrace(text: String): Boolean =
    elementType == DoriaTokenTypes.BRACE && this.text == text
