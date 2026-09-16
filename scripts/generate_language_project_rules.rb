#!/usr/bin/env ruby
# frozen_string_literal: true

# Generate marker-gated cleanup rules for language build/dependency artifacts.

require "yaml"

ROOT = File.expand_path("..", __dir__)
OUT = File.join(ROOT, "rules", "categories", "language-projects.yaml")

ROOTS = {
  "macos" => %w[
    $HOME/Projects $HOME/Source $HOME/Sources $HOME/Developer $HOME/src
    $HOME/repos $HOME/work $HOME/workspace $HOME/Documents/Projects
    $HOME/Documents/Source $HOME/Documents/Sources
  ],
  "linux" => %w[
    $HOME/Projects $HOME/Source $HOME/Sources $HOME/src $HOME/repos $HOME/code
    $HOME/work $HOME/workspace $HOME/Documents/Projects
    $HOME/Documents/Source $HOME/Documents/Sources
  ],
  "windows" => %w[
    %USERPROFILE%/source/repos %USERPROFILE%/Source %USERPROFILE%/Sources
    %USERPROFILE%/Projects %USERPROFILE%/src %USERPROFILE%/repos
    %USERPROFILE%/code %USERPROFILE%/work %USERPROFILE%/workspace
    %USERPROFILE%/Documents/Projects %USERPROFILE%/Documents/Source
    %USERPROFILE%/Documents/Sources
  ]
}.freeze

FAMILIES = [
  {
    id: "python-cache", name: "Python project caches",
    description: "Removes bytecode and tool caches; Python source, environments, and dependency files are preserved.",
    dirs: %w[**/__pycache__ **/.pytest_cache **/.mypy_cache **/.ruff_cache],
    markers: %w[pyproject.toml setup.py requirements.txt Pipfile], risk: "low", kind: "cache"
  },
  {
    id: "python-env", name: "Python disposable environments",
    description: "Removes recreatable local virtual/test environments; review first because reinstalling dependencies may take time.",
    dirs: %w[**/.venv **/.tox **/.nox],
    markers: %w[pyproject.toml requirements.txt Pipfile poetry.lock uv.lock],
    risk: "medium", kind: "packages"
  },
  {
    id: "rust", name: "Rust build output",
    description: "Removes Cargo target directories; Rust source and Cargo lockfiles are preserved.",
    dirs: %w[**/target], markers: %w[Cargo.toml], risk: "low", kind: "dev"
  },
  {
    id: "java-kotlin", name: "Java and Kotlin build output",
    description: "Removes Maven/Gradle build output and project-local Gradle caches; source and build definitions are preserved.",
    dirs: %w[**/target **/build **/.gradle],
    markers: %w[pom.xml build.gradle build.gradle.kts settings.gradle settings.gradle.kts],
    risk: "low", kind: "dev"
  },
  {
    id: "go", name: "Go project binaries",
    description: "Removes project-local bin output in Go modules; source and go.mod/go.sum are preserved.",
    dirs: %w[**/bin], markers: %w[go.mod], risk: "low", kind: "dev"
  },
  {
    id: "cpp", name: "C and C++ build output",
    description: "Removes conventional CMake/Meson build directories and clangd cache; source and project definitions are preserved.",
    dirs: %w[**/build **/cmake-build-* **/.cache/clangd],
    markers: %w[CMakeLists.txt meson.build], risk: "low", kind: "dev"
  },
  {
    id: "swift", name: "Swift Package Manager build output",
    description: "Removes SwiftPM .build directories; Package.swift, sources, and resolved versions are preserved.",
    dirs: %w[**/.build], markers: %w[Package.swift], risk: "low", kind: "dev"
  },
  {
    id: "ruby", name: "Ruby bundled dependencies",
    description: "Removes vendor/bundle dependencies when Gemfile.lock is present; application source and lockfile are preserved.",
    dirs: %w[**/vendor/bundle], markers: %w[Gemfile.lock],
    risk: "medium", kind: "packages"
  },
  {
    id: "php", name: "PHP Composer dependencies",
    description: "Removes Composer vendor dependencies when composer.lock is present; PHP source and lockfile are preserved.",
    dirs: %w[**/vendor], markers: %w[composer.lock], risk: "medium", kind: "packages"
  },
  {
    id: "dart-flutter", name: "Dart and Flutter generated output",
    description: "Removes .dart_tool and build directories; Dart source, pubspec, and lockfile are preserved.",
    dirs: %w[**/.dart_tool **/build], markers: %w[pubspec.yaml],
    risk: "low", kind: "dev"
  },
  {
    id: "elixir", name: "Elixir build output and dependencies",
    description: "Removes Mix _build and deps directories; Elixir source and mix.lock are preserved.",
    dirs: %w[**/_build **/deps], markers: %w[mix.exs], risk: "medium", kind: "packages"
  },
  {
    id: "haskell", name: "Haskell build output",
    description: "Removes Cabal and Stack build directories; Haskell source and project definitions are preserved.",
    dirs: %w[**/dist-newstyle **/.stack-work],
    markers: %w[stack.yaml cabal.project *.cabal], risk: "low", kind: "dev"
  },
  {
    id: "godot", name: "Godot imported project cache",
    description: "Removes the regenerable .godot import/cache directory; scenes, scripts, and project.godot are preserved.",
    dirs: %w[**/.godot], markers: %w[project.godot], risk: "low", kind: "cache"
  },
  {
    id: "unity", name: "Unity generated project directories",
    description: "Removes regenerable Library, Temp, Logs, and obj directories only in a Unity project; Assets and ProjectSettings are preserved.",
    dirs: %w[**/Library **/Temp **/Logs **/obj],
    markers: %w[ProjectSettings], risk: "medium", kind: "dev"
  }
].freeze

rules = []
ROOTS.each do |platform, roots|
  FAMILIES.each do |family|
    rules << {
      "id" => "project-#{family[:id]}-#{platform}",
      "name" => family[:name].dup,
      "description" => family[:description].dup,
      "category" => "dev",
      "platform" => platform,
      "risk" => family[:risk],
      "kind" => family[:kind],
      "default_enabled" => family[:risk] == "low",
      "min_age_days" => 0,
      "paths" => roots.map(&:dup),
      "include_globs" => family[:dirs].map(&:dup),
      "exclude_globs" => ["**/.git/**", "**/node_modules/**"],
      "required_ancestor_globs" => family[:markers].map(&:dup),
      "match_directories" => true,
      "delete_mode" => "contents",
      "max_depth" => 18,
      "tags" => [family[:id], "build-output", "project"]
    }
  end
end

File.write(OUT, YAML.dump({
  "pack" => "language-projects",
  "description" => "Generated marker-gated build and dependency cleanup rules.",
  "rules" => rules
}))
puts "#{OUT}: #{rules.length} rules"
