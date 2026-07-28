package dev.doria.intellij.psi

import com.intellij.psi.tree.IElementType
import dev.doria.intellij.DoriaLanguage

class DoriaElementType(debugName: String) : IElementType(debugName, DoriaLanguage)

object DoriaElementTypes {
    val BLOCK = DoriaElementType("DORIA_BLOCK")
    val PARENTHESIZED = DoriaElementType("DORIA_PARENTHESIZED")
    val BRACKETED = DoriaElementType("DORIA_BRACKETED")
}
