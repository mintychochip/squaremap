import com.github.jengelman.gradle.plugins.shadow.tasks.ShadowJar
import net.kyori.indra.git.IndraGitExtension
import org.gradle.api.Project
import org.gradle.api.file.ProjectLayout
import org.gradle.api.file.RegularFile
import org.gradle.api.plugins.BasePluginExtension
import org.gradle.api.provider.Provider
import org.gradle.api.provider.ProviderFactory
import org.gradle.kotlin.dsl.findByType
import org.gradle.kotlin.dsl.getByType
import java.io.File

internal data class GitCommandResult(
  val exitCode: Int,
  val output: String,
)

internal typealias GitCommandRunner = (List<String>, File) -> GitCommandResult
private val gitObjectIdCommand = listOf("git", "rev-parse", "--verify", "HEAD")

internal fun resolveNativeGitCommit(
  projectDir: File,
  runGit: GitCommandRunner = ::runGitCommand,
): String? {
  val result = try {
    runGit(gitObjectIdCommand, projectDir)
  } catch (interrupted: InterruptedException) {
    Thread.currentThread().interrupt()
    return null
  } catch (_: Exception) {
    return null
  }

  if (result.exitCode != 0) {
    return null
  }

  return result.output.trim().takeIf(::isFullGitObjectId)
}

private fun runGitCommand(command: List<String>, workingDirectory: File): GitCommandResult {
  val process = ProcessBuilder(command)
    .directory(workingDirectory)
    .redirectErrorStream(true)
    .start()
  val output = process.inputStream.bufferedReader().use { it.readText() }
  return GitCommandResult(process.waitFor(), output)
}

private fun isFullGitObjectId(value: String): Boolean =
  (value.length == 40 || value.length == 64) && value.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }

fun runProps(layout: ProjectLayout, providers: ProviderFactory): Map<String, String> = buildMap {
  put("squaremap.devFrontend", providers.gradleProperty("devFrontend").getOrElse("true"))
  put("squaremap.frontendPath", layout.settingsDirectory.dir("web").asFile.absolutePath)
  localBackendBinary(layout)?.let { put("squaremap.backendBinary", it) }
}

internal fun localBackendBinary(layout: ProjectLayout): String? {
  val os = System.getProperty("os.name").lowercase()
  val arch = System.getProperty("os.arch").lowercase()
  val windows = os.contains("windows")
  val triple = when {
    os.contains("linux") && (arch == "amd64" || arch == "x86_64") -> "x86_64-unknown-linux-gnu"
    os.contains("linux") && (arch == "aarch64" || arch == "arm64") -> "aarch64-unknown-linux-gnu"
    (os.contains("mac") || os.contains("darwin")) && (arch == "aarch64" || arch == "arm64") -> "aarch64-apple-darwin"
    (os.contains("mac") || os.contains("darwin")) && (arch == "amd64" || arch == "x86_64") -> "x86_64-apple-darwin"
    windows && (arch == "amd64" || arch == "x86_64") -> "x86_64-pc-windows-msvc"
    else -> return null
  }
  val name = "squaremap-server-$triple${if (windows) ".exe" else ""}"
  val file = layout.settingsDirectory.file("rust/backend/rust-backend-$triple/$name").asFile
  return file.takeIf { it.isFile }?.absolutePath
}

val Project.releaseNotes: Provider<String>
  get() = providers.environmentVariable("RELEASE_NOTES")

val Project.githubUrl: Provider<String>
  get() = providers.gradleProperty("githubUrl")

fun Project.lastCommitHash(): String {
  val indraCommit = extensions.findByType<IndraGitExtension>()?.commit()?.orNull?.name
  return indraCommit?.substring(0, 7)
    ?: resolveNativeGitCommit(rootProject.projectDir) { command, workingDirectory ->
      val output = providers.exec {
        commandLine(command)
        workingDir(workingDirectory)
        isIgnoreExitValue = true
      }
      GitCommandResult(
        exitCode = output.result.get().exitValue,
        output = output.standardOutput.asText.get(),
      )
    }?.substring(0, 7)
    ?: error("Could not determine commit hash")
}

fun Project.decorateVersion() {
  val versionString = version as String
  version = if (versionString.endsWith("-SNAPSHOT")) {
    "$versionString+${lastCommitHash()}"
  } else {
    versionString
  }
}

fun ShadowJar.reloc(pkg: String) {
  relocate(pkg, "squaremap.libraries.$pkg")
}

fun Project.currentBranch(): String {
  System.getenv("GITHUB_HEAD_REF")?.takeIf { it.isNotEmpty() }
    ?.let { return it }
  System.getenv("GITHUB_REF")?.takeIf { it.isNotEmpty() }
    ?.let { return it.replaceFirst("refs/heads/", "") }

  val indraGit = extensions.getByType<IndraGitExtension>().takeIf { it.isPresent }

  return indraGit?.branchName()?.orNull ?: "detached-head"
}

fun Project.productionJarName(mcVer: Provider<String>): Provider<String> = extensions.getByType<BasePluginExtension>()
  .archivesName.zip(mcVer) { archivesName, mc -> "$archivesName-mc$mc-$version.jar" }

fun Project.productionJarLocation(mcVer: Provider<String>): Provider<RegularFile> =
  productionJarName(mcVer).flatMap { layout.buildDirectory.file("libs/$it") }
