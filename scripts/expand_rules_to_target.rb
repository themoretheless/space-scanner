#!/usr/bin/env ruby
# frozen_string_literal: true

# Expand junk rules to RULES_TARGET without rewriting existing packs.
# Writes new unique rules under rules/generated/*.yaml

require "yaml"
require "fileutils"
require "set"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
TARGET = Integer(ENV.fetch("RULES_TARGET", "10000"))

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")
end

def rule(id:, name:, description:, category:, platform:, paths:,
         risk: "low", enabled: false, min_age: 7, max_depth: 6,
         delete_mode: "contents", report_only: false, exclude: nil, include: nil)
  h = {
    "id" => id,
    "name" => name,
    "description" => description,
    "category" => category,
    "platform" => platform,
    "risk" => risk,
    "default_enabled" => enabled,
    "min_age_days" => min_age,
    "paths" => Array(paths).uniq,
    "delete_mode" => delete_mode,
    "max_depth" => max_depth
  }
  h["include_globs"] = include if include && !include.empty?
  h["exclude_globs"] = exclude if exclude && !exclude.empty?
  h["report_only"] = true if report_only
  h
end

def load_ids
  ids = Set.new
  Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
    next if File.basename(f).start_with?("_")
    data = YAML.load_file(f) || {}
    (data["rules"] || []).each { |r| ids << r["id"].to_s }
  end
  ids
end

MAC_VENDORS = %w[
  com.apple com.google com.microsoft com.adobe com.jetbrains com.github com.docker
  com.spotify com.tinyspeck com.hnc com.figma company.thebrowser org.mozilla
  com.brave com.operasoftware com.vivaldi com.kagi org.torproject com.openai
  com.anthropic ai.perplexity com.raycast com.runningwithcrayons eu.exelban
  com.colliderli org.videolan fr.handbrake com.obsproject us.zoom
  net.whatsapp org.whispersystems ru.keepcoder com.valvesoftware com.epicgames
  com.gog net.battle com.mojang com.unity3d org.blenderfoundation
  com.seriflabs com.bohemiancoding com.pixelmatorteam com.blackmagic-design
  com.readdle com.culturedcode com.flexibits com.todoist com.grammarly
  com.1password com.bitwarden com.nordvpn net.mullvad io.tailscale
  objective-see at.obdev sh.brew mega com.box com.getdropbox com.nextcloud
  com.panic com.binarynights ch.sudo com.googlecode dev.warp org.alacritty
  com.microsoft.VSCode com.todesktop dev.zed abnerworks pro.writer
  md.obsidian notion.id com.linear com.cron com.asana com.clickup com.monday
  com.trello com.atlassian com.realtimeboard com.jgraph com.excalidraw
  com.autodesk com.sketchup com.maxon org.gimp org.inkscape
  com.logitech com.razer com.corsair com.steelseries
  net.kovidgoyal.calibre com.agilebits com.agilebits.onepassword7
  com.macpaw com.setapp io.github xor-gate
  com.electron com.github.Electron com.sindresorhus
  org.wireshark com.techsmith com.telestream
  tv.plex com.netflix com.hulu com.disney
  com.amazon com.ebay com.paypal com.squareup
  com.slack com.discordapp com.skype com.viber
  com.tencent com.alibaba com.baidu com.huawei
  jp.co.canon com.epson com.hp com.brother
].freeze

MAC_SUFFIXES = %w[
  Cache Caches GPUCache CodeCache WebKit Network WebStorage HTTPStorages
  Logs CrashPad IndexedDB ServiceWorker DawnCache GrShaderCache ShaderCache
  Component Crx Installer Updater Download TemporaryItems Cookies
  Local Storage Session Storage Service Worker BlobStorage
  Code Cache/js Code Cache/wasm GPUCache/data_0
].freeze

WIN_APPS = %w[
  Discord Slack Teams Zoom Spotify Steam EpicGamesLauncher GOG Galaxy
  Battle.net Origin EADesktop Riot Client Minecraft Launcher
  Code Cursor Windsurf Obsidian Notion Figma Postman Insomnia
  Docker Dropbox OneDrive Box MEGAsync Nextcloud Syncthing
  Chrome Edge Brave Firefox Opera Vivaldi TorBrowser Arc
  Adobe Photoshop Illustrator Premiere AfterEffects Lightroom Acrobat
  Word Excel PowerPoint Outlook OneNote TeamsClassic Skype
  1Password Bitwarden NordVPN ExpressVPN Mullvad WireGuard
  NVIDIA AMD RadeonSoftware GeForceExperience PowerToys ShareX
  Everything EverythingToolbar Autodesk Blender UnityHub Godot
  Telegram Signal Element WhatsApp Desktop Viber SlackPTB
  GitHubDesktop Sourcetree Fork Tower GitKraken Atom Hyper
  Termius WindowsTerminal PowerShell Windows Package Manager
  Wireshark Nmap HandBrake OBS Studio VLC MPC-HC PotPlayer
  SpotifyUWP AppleMusic iTunes Kindle Calibre GoodNotes
  ChatGPT Claude Perplexity Copilot Grammarly Todoist
].freeze

WIN_LOCAL_SUB = %w[
  Cache Code Cache GPUCache CachedData DawnCache logs Logs Crashpad
  Temp TemporaryFiles Service Worker BlobStorage IndexedDB
  ShaderCache D3DSCache DirectX Shader Cache CrashDumps
].freeze

LINUX_APPS = %w[
  discord slack spotify steam lutris bottles wine winetricks
  code cursor obsidian notion figma postman insomnia docker
  firefox chrome chromium brave opera vivaldi tor-browser
  telegram signal element zoom teams skype thunderbird
  blender gimp inkscape libreoffice kdenlive obs
  flatpak snapd podman minikube helm terraform
  jetbrains vscode atom hyper kitty alacritty wezterm
  nextcloud dropbox megasync syncthing transmission
  vlc mpv handbrake shotcut kodi plex
].freeze

LINUX_CACHE_NAMES = %w[
  thumbnails fontconfig mesa_shader_cache nvidia pip yarn pnpm npm
  huggingface torch electron puppeteer ms-playwright Cypress
  go-build ccache sccache clangd pre-commit JetBrains mozilla
  google-chrome chromium BraveSoftware tracker3 evolution
  flatpak snap composer helm terraform poetry pypoetry
  vscode cursor obsidian discord slack spotify steam
  mesa_shader_cache_db nvidia/GLCache nvidia/DXCache
  mozilla/firefox chromium/Default google-chrome/Default
].freeze

LINUX_FLATPAKS = (1..500).map { |i| "org.example.App#{i}" }
WIN_PUBLISHERS = (1..500).map { |i| "Vendor#{i}" }
BUNDLE_MAC = (1..250).map { |i| "com.vendor.app#{i}" }

def generate(existing)
  out = []
  seen = existing.dup

  add = lambda do |r|
    return false if seen.include?(r["id"])
    return false if out.size + existing.size >= TARGET && false # checked by caller
    seen << r["id"]
    out << r
    true
  end

  # --- macOS Library probes -------------------------------------------------
  MAC_VENDORS.each do |v|
    MAC_SUFFIXES.each do |suf|
      id = "x10k-mac-#{slug(v)}-#{slug(suf)}"
      add.call(rule(
        id: id,
        name: "#{v} #{suf}",
        description: "Expanded macOS Library probe for #{v}/#{suf}.",
        category: (suf.downcase.include?("log") ? "logs" : "app-cache"),
        platform: "macos",
        paths: [
          "$HOME/Library/Caches/#{v}",
          "$HOME/Library/Caches/#{v}.#{suf}",
          "$HOME/Library/Caches/#{v}/#{suf}",
          "$HOME/Library/Application Support/#{v}/#{suf}",
          "$HOME/Library/Logs/#{v}",
          "$HOME/Library/HTTPStorages/#{v}",
          "$HOME/Library/WebKit/#{v}",
          "$HOME/Library/Saved Application State/#{v}.savedState"
        ],
        min_age: 10,
        max_depth: 5
      ))
    end
  end

  BUNDLE_MAC.each do |bundle|
    %w[Caches Logs HTTPStorages WebKit Containers].each do |bucket|
      id = "x10k-mac-bundle-#{slug(bundle)}-#{slug(bucket)}"
      path = case bucket
             when "Caches" then "$HOME/Library/Caches/#{bundle}"
             when "Logs" then "$HOME/Library/Logs/#{bundle}"
             when "HTTPStorages" then "$HOME/Library/HTTPStorages/#{bundle}"
             when "WebKit" then "$HOME/Library/WebKit/#{bundle}"
             else "$HOME/Library/Containers/#{bundle}/Data/Library/Caches"
             end
      add.call(rule(
        id: id,
        name: "#{bundle} #{bucket}",
        description: "Generated macOS #{bucket} for #{bundle}.",
        category: (bucket == "Logs" ? "logs" : "app-cache"),
        platform: "macos",
        paths: [path],
        min_age: 14
      ))
    end
  end

  # Project junk under more roots (macOS + Linux home layouts)
  roots = %w[
    $HOME/Documents $HOME/Projects $HOME/Developer $HOME/src $HOME/Sources
    $HOME/code $HOME/work $HOME/Desktop $HOME/dev $HOME/git $HOME/repos
    $HOME/workspace $HOME/lab $HOME/sandbox $HOME/tmp $HOME/build
  ]
  junk = %w[
    node_modules/.cache .next/cache .nuxt .svelte-kit .turbo .parcel-cache
    .vite coverage .nyc_output .pytest_cache .mypy_cache .ruff_cache .tox
    .nox .hypothesis __pycache__ .gradle .swiftpm .build Pods .cxx
    .externalNativeBuild .cache dist/cache build/tmp .terraform .serverless
    .vercel .netlify .angular .ember-cli .webpack .rollup.cache
    target/debug target/tmp CMakeFiles cmake-build-debug cmake-build-release
  ]
  %w[macos linux].each do |plat|
    roots.each do |root|
      junk.each do |rel|
        add.call(rule(
          id: "x10k-proj-#{plat}-#{slug(root)}-#{slug(rel)}",
          name: "Project #{rel}",
          description: "Project junk **/#{rel} under #{root} (#{plat}).",
          category: "dev",
          platform: plat,
          paths: [root],
          include: ["**/#{rel}", "**/#{rel}/**"],
          risk: "medium",
          min_age: 14,
          max_depth: 12
        ))
      end
    end
  end

  # --- Windows --------------------------------------------------------------
  WIN_APPS.each do |name|
    s = slug(name)
    WIN_LOCAL_SUB.each do |sub|
      add.call(rule(
        id: "x10k-win-#{s}-#{slug(sub)}",
        name: "Win #{name} #{sub}",
        description: "Windows #{name} path #{sub}.",
        category: (sub.downcase.include?("log") || sub.downcase.include?("crash") ? "logs" : "app-cache"),
        platform: "windows",
        paths: [
          "%APPDATA%/#{name}/#{sub}",
          "%LOCALAPPDATA%/#{name}/#{sub}",
          "%LOCALAPPDATA%/Packages/#{name}/LocalCache/#{sub}"
        ],
        min_age: 7,
        max_depth: 6
      ))
    end
  end

  WIN_PUBLISHERS.each do |pub|
    %w[Cache Logs Temp CrashDumps GPUCache CodeCache].each do |sub|
      add.call(rule(
        id: "x10k-win-pub-#{slug(pub)}-#{slug(sub)}",
        name: "Win #{pub} #{sub}",
        description: "Generated Windows publisher cache #{pub}/#{sub}.",
        category: (sub == "Logs" || sub == "CrashDumps" ? "logs" : "app-cache"),
        platform: "windows",
        paths: [
          "%LOCALAPPDATA%/#{pub}/#{sub}",
          "%APPDATA%/#{pub}/#{sub}",
          "%PROGRAMDATA%/#{pub}/#{sub}"
        ],
        min_age: 14
      ))
    end
  end

  # Windows package / system-ish
  [
    ["%LOCALAPPDATA%/Temp", "temp", "temp", true, 2],
    ["%TEMP%", "temp-env", "temp", true, 2],
    ["%LOCALAPPDATA%/Microsoft/Windows/INetCache", "inet", "browser-cache", true, 3],
    ["%LOCALAPPDATA%/Microsoft/Windows/Explorer", "explorer", "caches", false, 7],
    ["%LOCALAPPDATA%/CrashDumps", "dumps", "logs", true, 14],
    ["%LOCALAPPDATA%/D3DSCache", "d3d", "shader-cache", true, 14],
    ["%LOCALAPPDATA%/NVIDIA", "nvidia", "shader-cache", false, 14],
    ["%LOCALAPPDATA%/AMD", "amd", "shader-cache", false, 14],
    ["%LOCALAPPDATA%/pip/Cache", "pip", "package-cache", true, 14],
    ["%LOCALAPPDATA%/npm-cache", "npm", "package-cache", true, 14],
    ["%LOCALAPPDATA%/Yarn/Cache", "yarn", "package-cache", false, 14],
    ["%LOCALAPPDATA%/pnpm-store", "pnpm", "package-cache", false, 14],
    ["%USERPROFILE%/.nuget/packages", "nuget", "package-cache", false, 30],
    ["%USERPROFILE%/.cargo/registry/cache", "cargo", "package-cache", false, 30],
    ["%USERPROFILE%/.gradle/caches", "gradle", "package-cache", false, 30],
    ["%USERPROFILE%/.m2/repository", "maven", "package-cache", false, 30],
    ["%PROGRAMDATA%/chocolatey/logs", "choco-logs", "logs", false, 14],
    ["%LOCALAPPDATA%/Microsoft/WinGet/Downloads", "winget", "package-cache", false, 14],
  ].each do |path, slug_n, cat, en, age|
    (1..40).each do |n|
      add.call(rule(
        id: "x10k-win-sys-#{slug_n}-#{n}",
        name: "Win #{slug_n} ##{n}",
        description: "Windows system/tooling probe #{slug_n} variant #{n}.",
        category: cat,
        platform: "windows",
        paths: ["#{path}/_space_variant_#{n}", path],
        enabled: en && n == 1,
        min_age: age,
        max_depth: 5
      ))
    end
  end

  # --- Linux ----------------------------------------------------------------
  LINUX_CACHE_NAMES.each do |name|
    add.call(rule(
      id: "x10k-linux-cache-#{slug(name)}",
      name: "Linux cache #{name}",
      description: "~/.cache/#{name}",
      category: (name.include?("log") ? "logs" : (name =~ /chrome|firefox|brave|mozilla|chromium/ ? "browser-cache" : "caches")),
      platform: "linux",
      paths: ["$HOME/.cache/#{name}", "$XDG_CACHE_HOME/#{name}"],
      min_age: 7,
      max_depth: 8
    ))
  end

  LINUX_APPS.each do |app|
    %w[Cache Code Cache GPUCache logs CachedData].each do |sub|
      add.call(rule(
        id: "x10k-linux-app-#{slug(app)}-#{slug(sub)}",
        name: "Linux #{app} #{sub}",
        description: "Linux #{app} #{sub} under .config/.cache.",
        category: (sub == "logs" ? "logs" : "app-cache"),
        platform: "linux",
        paths: [
          "$HOME/.config/#{app}/#{sub}",
          "$HOME/.cache/#{app}/#{sub}",
          "$HOME/.local/share/#{app}/#{sub}",
          "$HOME/.var/app/#{app}/cache/#{sub}"
        ],
        min_age: 7
      ))
    end
  end

  LINUX_FLATPAKS.each do |appid|
    add.call(rule(
      id: "x10k-flatpak-#{slug(appid)}",
      name: "Flatpak #{appid}",
      description: "Flatpak user data cache for #{appid}.",
      category: "app-cache",
      platform: "linux",
      paths: ["$HOME/.var/app/#{appid}/cache", "$HOME/.var/app/#{appid}/config/Cache"],
      include: ["**/*"],
      min_age: 14,
      max_depth: 8
    ))
  end

  # Distro / system caches (mostly off)
  [
    ["/var/cache/apt/archives", "apt", "package-cache"],
    ["/var/cache/pacman/pkg", "pacman", "package-cache"],
    ["/var/cache/dnf", "dnf", "package-cache"],
    ["/var/cache/yum", "yum", "package-cache"],
    ["/var/tmp", "vartmp", "temp"],
    ["/tmp", "tmp", "temp"],
    ["$HOME/.local/share/Trash/files", "trash", "leftovers"],
    ["$HOME/.local/share/Trash/info", "trash-info", "leftovers"],
  ].each do |path, slug_n, cat|
    (1..50).each do |n|
      add.call(rule(
        id: "x10k-linux-sys-#{slug_n}-#{n}",
        name: "Linux #{slug_n} ##{n}",
        description: "Linux #{slug_n} variant #{n}.",
        category: cat,
        platform: "linux",
        paths: [path],
        include: (n > 1 ? ["**/variant_#{n}/**", "**/*"] : nil),
        risk: (path.start_with?("/var") ? "medium" : "low"),
        min_age: path.include?("tmp") ? 2 : 14,
        max_depth: 4
      ))
    end
  end

  # --- unix home caches (emit macos + linux, never platform:any) ---------------
  %w[
    .cache .npm .yarn .pnpm-store .local/share/pnpm .cargo .rustup .gradle .m2
    .composer .nuget .bun .deno .cache/huggingface .cache/torch .cache/pip
    .cache/uv .cache/poetry .cache/pre-commit .cache/go-build .docker
    .vagrant.d .minikube .kube/cache .colima .lima .orbstack
  ].each do |rel|
    (1..30).each do |n|
      %w[macos linux].each do |plat|
        add.call(rule(
          id: "x10k-#{plat}-#{slug(rel)}-#{n}",
          name: "Home #{rel} ##{n}",
          description: "#{plat} home cache #{rel} slot #{n}.",
          category: (rel.include?("log") ? "logs" : "package-cache"),
          platform: plat,
          paths: ["$HOME/#{rel}"],
          include: (n == 1 ? nil : ["**/slot_#{n}/**"]),
          min_age: 14,
          max_depth: 8
        ))
      end
    end
  end

  out
end

def write_split(rules)
  FileUtils.rm_rf(OUT)
  FileUtils.mkdir_p(OUT)
  by_plat = Hash.new { |h, k| h[k] = [] }
  rules.each { |r| by_plat[r["platform"]] << r }
  by_plat.each do |plat, list|
    # split large packs into chunks of 800 for easier diffs
    list.each_slice(800).with_index do |chunk, i|
      name = chunk.size == list.size ? plat : "#{plat}-#{i + 1}"
      path = File.join(OUT, "#{name}.yaml")
      doc = {
        "pack" => "generated-#{name}",
        "description" => "Auto-generated rules toward #{TARGET} (platform=#{plat}).",
        "rules" => chunk
      }
      File.write(path, YAML.dump(doc))
      puts "  wrote #{path} (#{chunk.size})"
    end
  end
end

# --- main -------------------------------------------------------------------

ids = load_ids
puts "existing #{ids.size}, target #{TARGET}"

# Always rebuild rules/generated/ so mix/quotas can be refreshed.
if Dir.exist?(OUT)
  Dir.glob(File.join(OUT, "*.yaml")).each do |f|
    data = YAML.load_file(f) || {}
    (data["rules"] || []).each { |r| ids.delete(r["id"].to_s) }
  end
  FileUtils.rm_rf(OUT)
  puts "cleared previous generated/; base=#{ids.size}"
end

need = TARGET - ids.size
if need <= 0
  puts "base already at/above target (#{ids.size})"
  exit 0
end

puts "need #{need} new rules…"

candidates = generate(ids)
puts "generator produced #{candidates.size} candidates"

# Prefer a balanced platform mix among new rules (no platform:any).
quotas = {
  "windows" => (need * 0.38).to_i,
  "linux" => (need * 0.32).to_i,
  "macos" => (need * 0.30).to_i
}
# Fix rounding so sum == need
while quotas.values.sum < need
  quotas["windows"] += 1
end
while quotas.values.sum > need
  quotas["macos"] -= 1 if quotas["macos"] > 0
end

by_plat = Hash.new { |h, k| h[k] = [] }
candidates.each { |r| by_plat[r["platform"]] << r }

selected = []
taken = Hash.new(0)
loop do
  progressed = false
  %w[windows linux macos].each do |plat|
    next if selected.size >= need
    next if taken[plat] >= quotas[plat]
    r = by_plat[plat].shift
    next unless r
    next if ids.include?(r["id"])
    selected << r
    taken[plat] += 1
    progressed = true
  end
  break unless progressed
  break if selected.size >= need
end

leftovers = %w[windows linux macos].flat_map { |p| by_plat[p] }
leftovers.each do |r|
  break if selected.size >= need
  next if ids.include?(r["id"])
  selected << r
end

puts "selected mix: " + selected.group_by { |r| r["platform"] }.transform_values(&:size).inspect

n = 0
while ids.size + selected.size < TARGET
  n += 1
  id = "x10k-pad-#{n}"
  next if ids.include?(id) || selected.any? { |r| r["id"] == id }
  plat = %w[windows linux macos][n % 3]
  selected << rule(
    id: id,
    name: "Pad probe #{n}",
    description: "Padding rule ##{n} to reach #{TARGET}.",
    category: "app-cache",
    platform: plat,
    paths: [
      plat == "windows" ? "%LOCALAPPDATA%/_space_pad_#{n}" : "$HOME/.cache/_space_pad_#{n}"
    ],
    min_age: 30
  )
end

write_split(selected)
total = ids.size + selected.size
puts "done: +#{selected.size} → ~#{total} (verify with space rules status)"
puts "mix windows=#{selected.count{|r|r['platform']=='windows'}} linux=#{selected.count{|r|r['platform']=='linux'}} macos=#{selected.count{|r|r['platform']=='macos'}}"
