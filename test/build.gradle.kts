plugins {
  id("squaremap.base-conventions")
  alias(libs.plugins.resource.factory.bukkit)
}

description = "Test plugin for exercising the squaremap API against a live Paper server"

val minecraftVersion = libs.versions.minecraft
val plainMinecraftVersion = minecraftVersion.get()
  .split("[.-]".toRegex())
  .mapNotNull { s -> s.toIntOrNull() }
  .joinToString(".")

dependencies {
  compileOnly(projects.squaremapApi)
  compileOnly(libs.paperApi)
  compileOnly(libs.checkerQual)
}

bukkitPluginYaml {
  name = "squaremap-test"
  main = "xyz.jpenilla.squaremap.test.SquaremapTestPlugin"
  apiVersion = plainMinecraftVersion
  authors = listOf("jmp")
  website = githubUrl
  foliaSupported = true
  depend = listOf("squaremap")
  commands {
    register("squaremaptest") {
      description = "Inspect the squaremap API state from the test plugin"
      permission = "squaremap-test.command"
    }
  }
}
