import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class ExtTest {
  @Test
  fun `native git resolver validates and trims a full object id`() {
    val root = File("/linked-worktree")
    val objectId = "a".repeat(40)
    val result = resolveNativeGitCommit(root) { command, workingDirectory ->
      assertEquals(listOf("git", "rev-parse", "--verify", "HEAD"), command)
      assertEquals(root, workingDirectory)
      GitCommandResult(exitCode = 0, output = "  $objectId\n")
    }

    assertEquals(objectId, result)
  }

  @Test
  fun `native git resolver accepts a sixty-four character object id`() {
    val objectId = "b".repeat(64)

    assertEquals(
      objectId,
      resolveNativeGitCommit(File("/linked-worktree")) { _, _ ->
        GitCommandResult(exitCode = 0, output = "$objectId\n")
      },
    )
  }

  @Test
  fun `native git resolver rejects unsuccessful or malformed output`() {
    val root = File("/linked-worktree")
    val runGit: (List<String>, File) -> GitCommandResult = { _, _ ->
      GitCommandResult(exitCode = 1, output = "not a repository")
    }
    assertNull(resolveNativeGitCommit(root, runGit))

    val malformed = { _: List<String>, _: File ->
      GitCommandResult(exitCode = 0, output = "not-an-object-id\n")
    }
    assertNull(resolveNativeGitCommit(root, malformed))
  }
}
