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
  providers.systemProperty("squaremap.renderBenchmark").orNull?.let { systemProperty("squaremap.renderBenchmark", it) }
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
val backendArtifactDirectory = layout.buildDirectory.dir("backend")
val backendTargets = listOf(
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
)
val backendBinary = backendArtifactDirectory.map { it }
abstract class StageBackendBinary : DefaultTask() {
  @get:InputDirectory abstract val sourceDirectory: DirectoryProperty
  @get:OutputDirectory abstract val destinationDirectory: DirectoryProperty
  @get:Input abstract val targets: ListProperty<String>
  @TaskAction fun stage() {
    val source = sourceDirectory.get().asFile
    check(source.isDirectory) { "Download Rust backend artifacts before packaging: $source" }
    val destination = destinationDirectory.get().asFile
    destination.mkdirs()
    targets.get().forEach { target ->
      val sourceName = "squaremap-server-$target${if (target.contains("windows")) ".exe" else ""}"
      val artifact = source.resolve("rust-backend-$target").resolve(sourceName)
      check(artifact.isFile) { "Missing Rust backend artifact for $target: $artifact" }
      val targetDirectory = destination.resolve(target)
      targetDirectory.mkdirs()
      artifact.copyTo(targetDirectory.resolve(sourceName), overwrite = true)
    }
  }
}
val stageBackendBinary = tasks.register<StageBackendBinary>("stageBackendBinary") {
  sourceDirectory = rootProject.layout.projectDirectory.dir("rust/backend")
  destinationDirectory = backendArtifactDirectory
  targets = backendTargets
}
abstract class GenerateBackendManifest : DefaultTask() {
  @get:InputDirectory abstract val binariesDirectory: DirectoryProperty
  @get:OutputFile abstract val manifestFile: RegularFileProperty
  @get:Input abstract val targets: ListProperty<String>
  @get:Input abstract val artifactBaseUrl: Property<String>
  @get:Input abstract val pluginVersion: Property<String>
  @TaskAction fun generate() {
    val root = manifestFile.get().asFile
    root.parentFile.mkdirs()
    val binaries = targets.get().map { target ->
      val binary = binariesDirectory.get().asFile.resolve(target).resolve("squaremap-server-$target${if (target.contains("windows")) ".exe" else ""}")
      check(binary.isFile) { "Missing Rust backend artifact for $target: $binary" }
      target to binary
    }
    val digests = binaries.map { (_, binary) ->
      MessageDigest.getInstance("SHA-256").digest(binary.readBytes()).joinToString("") { byte: Byte -> "%02x".format(byte.toInt() and 0xff) }
    }
    check(digests.distinct().size == digests.size) {
      "Rust backend artifacts must have distinct SHA-256 digests; placeholder copies cannot form a release manifest"
    }
    val entries = binaries.joinToString(",\n") { (target, binary) ->
      val bytes = binary.readBytes()
      val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { byte: Byte -> "%02x".format(byte.toInt() and 0xff) }
      """    "$target": {
      "url": "${artifactBaseUrl.get()}${binary.name}",
      "length": "${bytes.size}",
      "sha256": "$digest"
    }"""
    }
    root.writeText("""{
  "pluginVersion": "${pluginVersion.get()}",
  "targets": {
$entries
  }
}
""")
  }
}
val generateBackendManifest = tasks.register<GenerateBackendManifest>("generateBackendManifest") {
  manifestFile = layout.buildDirectory.file("generated-resources/squaremap-backends.json")
  binariesDirectory = backendArtifactDirectory
  targets = backendTargets
  artifactBaseUrl = providers.gradleProperty("squaremap.backendArtifactBaseUrl").orElse(providers.provider { "https://github.com/jpenilla/squaremap/releases/download/v${project.version}/" })
  pluginVersion = project.version.toString()
  dependsOn(stageBackendBinary)
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
