package dev.doria.intellij.actions

import com.google.gson.JsonElement
import com.google.gson.JsonObject
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile

/** A type a new Doria declaration may name in its header. */
internal data class DoriaSupertype(
    /** The spelling the language server chose for the new file's namespace. */
    val label: String,
    val qualifiedName: String,
    val typeParameters: List<String>,
) {
    /**
     * The text written after `extends` or `implements`. A generic type keeps
     * its parameter names as arguments for the author to replace.
     */
    val headerText: String
        get() = if (typeParameters.isEmpty()) label else "$label<${typeParameters.joinToString(", ")}>"
}

/** The language server's answer for a declaration about to be created. */
internal data class DoriaSupertypeCandidates(
    /** The Baton package that owns the directory, or null outside every package. */
    val packageName: String?,
    val classes: List<DoriaSupertype>,
    val interfaces: List<DoriaSupertype>,
    /** The words the compiler's lexer reserves. */
    val keywords: Set<String>,
) {
    companion object {
        fun parse(response: JsonObject): DoriaSupertypeCandidates = DoriaSupertypeCandidates(
            packageName = response.get("package")?.takeUnless(JsonElement::isJsonNull)?.asString,
            classes = supertypes(response, "classes"),
            interfaces = supertypes(response, "interfaces"),
            keywords = response.getAsJsonArray("keywords").map(JsonElement::getAsString).toSet(),
        )

        private fun supertypes(response: JsonObject, field: String): List<DoriaSupertype> =
            response.getAsJsonArray(field).map { element ->
                val candidate = element.asJsonObject
                DoriaSupertype(
                    label = candidate.get("label").asString,
                    qualifiedName = candidate.get("qualifiedName").asString,
                    typeParameters = candidate.getAsJsonArray("typeParameters").map(JsonElement::getAsString),
                )
            }
    }
}

internal sealed interface DoriaSupertypeLookup {
    data class Found(val candidates: DoriaSupertypeCandidates) : DoriaSupertypeLookup

    /** Why the server could not answer, worded with what to do about it. */
    data class Unavailable(val reason: String) : DoriaSupertypeLookup
}

/**
 * Lists the types a new declaration may extend and implement. The LSP module
 * registers the implementation; an IDE without LSP support has none.
 */
internal interface DoriaSupertypeSource {
    /** Blocks on the language server; call it off the event dispatch thread. */
    fun request(directory: VirtualFile, fileName: String, namespace: String, development: Boolean): DoriaSupertypeLookup

    companion object {
        const val COMMAND = "doria.supertypeCandidates"

        fun lookup(project: Project, directory: VirtualFile, fileName: String, namespace: String, development: Boolean) =
            project.getService(DoriaSupertypeSource::class.java)
                ?.request(directory, fileName, namespace, development)
                ?: DoriaSupertypeLookup.Unavailable(
                    "Parent type lists and keyword checks need an IDE with LSP support, such as PhpStorm " +
                        "or IntelliJ IDEA Ultimate.",
                )
    }
}
