import java.security.MessageDigest

plugins {
  id("squaremap.base-conventions")
  id("net.neoforged.moddev")
  id("com.google.protobuf") version libs.versions.protobufPlugin.get()
}
neoForge {
  enable {
    neoFormVersion = libs.versions.neoform.get()
  }
  accessTransformers.from(layout.projectDirectory.file("src/main/resources/squaremap-common-at.cfg"))
}
sourceSets {
  main {
    proto {
      srcDir(rootProject.file("protocol"))
    }
  }
}

protobuf {
  protoc {
    artifact = libs.protoc.get().toString()
  }
}

sourceSets {
  test {
    java.srcDir("src/testFixtures/java")
  }
}

tasks.test {
  useJUnitPlatform()
  systemProperty("squaremap.task11.root", rootProject.projectDir.absolutePath)
  providers.systemProperty("squaremap.regenerate").orNull?.let { systemProperty("squaremap.regenerate", it) }
}

configurations.testCompileClasspath {
  extendsFrom(configurations.compileClasspath.get())
}

configurations.testRuntimeClasspath {
  extendsFrom(configurations.runtimeClasspath.get())
}

dependencies {
  api(libs.protobufJava)
  implementation(libs.aircompressor)
  testImplementation(libs.junitJupiter)
  testImplementation("com.google.code.gson:gson:2.13.1")
  testRuntimeOnly(libs.junitPlatformLauncher)
  testRuntimeOnly(tasks.named("createMinecraftArtifacts").map { it.outputs.files })
  testImplementation(libs.adventureApi)
  testImplementation(libs.miniMessage)
  api(projects.squaremapApi)
  api("com.google.inject:guice:${libs.versions.guice.get()}:classes") {
    exclude("com.google.guava")
  }
  api(libs.guiceAssistedInject) {
    exclude("com.google.guava")
  }

  api(platform(libs.adventureBom))
  compileOnlyApi(libs.adventureApi)
  compileOnlyApi(libs.adventureTextSerializerPlain)
  compileOnly(libs.adventureTextSerializerGson)
  compileOnlyApi(libs.miniMessage)

  api(platform(libs.cloudBom))
  api(platform(libs.cloudMinecraftBom))
  api(platform(libs.cloudProcessorsBom))
  api(libs.cloudCore)
  api(libs.cloudConfirmation)
  compileOnly(libs.cloudBrigadier)
  api(libs.cloudMinecraftExtras)

  api(platform(libs.configurateBom))
  api(libs.configurateYaml) {
    exclude("net.kyori", "option")
  }

  api(libs.undertow)

  api(libs.htmlSanitizer) {
    isTransitive = false
  }
  api(libs.htmlSanitizerJ8) {
    isTransitive = false
  }
  api(libs.htmlSanitizerJ10) {
    isTransitive = false
  }

  compileOnly("curse.maven:moonrise-1096335:8324171")
}

@UntrackedTask(because = "Up-to-date checking for this needs further thought")
abstract class BuildFrontend : DefaultTask() {
  @get:InputDirectory
  abstract val workingDir: DirectoryProperty
  @get:OutputDirectory
  abstract val outputDir: DirectoryProperty
  @get:Input
  abstract val command: ListProperty<String>

  @TaskAction
  fun run() {
    ProcessBuilder(this@BuildFrontend.command.get())
      .directory(this@BuildFrontend.workingDir.get().asFile)
      .inheritIO()
      .start()
      .waitFor()
      .also { check(it == 0) { "Frontend build exited with $it" } }
  }
}

val buildFrontend = tasks.register<BuildFrontend>("buildFrontend") {
  outputDir = layout.buildDirectory.dir("web")
  workingDir = layout.settingsDirectory.dir("web")
  val isWindows = System.getProperty("os.name").lowercase().contains("win")
  command = if (isWindows) {
    listOf("bun", "run", "build")
  } else {
    listOf("bash", "-c", "bun run build")
  }
}
val generateBackendManifest = tasks.register("generateBackendManifest") {
  val output = layout.buildDirectory.file("generated-resources/squaremap-backends.json")
  val backendBinary = rootProject.layout.projectDirectory.file("rust/target/release/squaremap-server")
  val pluginVersion = project.version.toString()
  inputs.file(backendBinary)
  outputs.file(output)
  doLast {
    check(backendBinary.asFile.isFile) {
      "Build the Rust backend first: ${backendBinary.asFile}"
    }
    val bytes = backendBinary.asFile.readBytes()
    val digest = MessageDigest.getInstance("SHA-256")
      .digest(bytes)
      .joinToString("") { byte: Byte -> "%02x".format(byte.toInt() and 0xff) }
    val target = when {
      System.getProperty("os.name").startsWith("Windows") -> "x86_64-pc-windows-msvc"
      System.getProperty("os.name").startsWith("Mac") && System.getProperty("os.arch") == "aarch64" -> "aarch64-apple-darwin"
      System.getProperty("os.name").startsWith("Mac") -> "x86_64-apple-darwin"
      System.getProperty("os.arch") == "aarch64" -> "aarch64-unknown-linux-gnu"
      else -> "x86_64-unknown-linux-gnu"
    }
    val root = output.get().asFile
    root.parentFile.mkdirs()
    root.writeText("""{
  "pluginVersion": "$pluginVersion",
  "targets": {
    "$target": {
      "url": "file://${backendBinary.asFile.absolutePath.replace("\\", "/")}",
      "length": "${bytes.size}",
      "sha256": "$digest"
    }
  }
}
""")
  }
}
tasks.processResources {
  duplicatesStrategy = DuplicatesStrategy.FAIL
  dependsOn(generateBackendManifest)
  from(generateBackendManifest) {
    into("")
  }
  from(buildFrontend.flatMap { it.outputDir }) {
    into("web")
  }
}
