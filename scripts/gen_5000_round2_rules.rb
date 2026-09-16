#!/usr/bin/env ruby
# frozen_string_literal: true

# Second import wave: Flathub, leftover Homebrew casks, Scoop buckets.
# Writes rules/generated/src5kb-*.yaml (does not rewrite src5k-*).

require "json"
require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
TARGET = 5000

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 80]
end

def load_ids
  ids = Set.new
  Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
    next if File.basename(f).start_with?("_")
    next if File.basename(f).start_with?("src5kb-")
    text = File.read(f)
    text.scan(/^- id: (.+)$/).each { |m| ids << m[0].strip }
  end
  ids
end

seen = load_ids
rules = []

add = lambda do |id:, name:, description:, category:, platform:, paths:, risk: "low", min_age: 10|
  return false if seen.include?(id)
  paths = Array(paths).map { |p| p.to_s.strip }.reject(&:empty?).uniq
  return false if paths.empty?
  seen << id
  rules << {
    "id" => id,
    "name" => name.to_s[0, 120],
    "description" => description.to_s[0, 400],
    "category" => category,
    "platform" => platform,
    "risk" => risk,
    "default_enabled" => false,
    "min_age_days" => min_age,
    "paths" => paths,
    "delete_mode" => "contents",
    "max_depth" => 6
  }
  true
end

# Flathub app IDs → user flatpak cache (Flathub API /api/v2/appstream)
if File.exist?("/tmp/flathub-appstream.json")
  JSON.parse(File.read("/tmp/flathub-appstream.json")).each do |appid|
    appid = appid.to_s
    next if appid.empty? || appid.include?("/")
    add.call(
      id: "fh-#{slug(appid)}",
      name: "Flatpak #{appid} cache",
      description: "User Flatpak cache for #{appid} from Flathub appstream catalog. Not ~/.var/app/#{appid}/data documents.",
      category: "app-cache",
      platform: "linux",
      paths: [
        "$HOME/.var/app/#{appid}/cache",
        "$HOME/.var/app/#{appid}/cache/tmp",
        "$HOME/.var/app/#{appid}/config/Cache",
        "$HOME/.var/app/#{appid}/config/GPUCache",
        "$HOME/.var/app/#{appid}/config/Code Cache"
      ],
      min_age: 7
    )
  end
end

# Scoop bucket apps (Windows)
%w[/tmp/scoop-ScoopInstaller_Main.txt /tmp/scoop-ScoopInstaller_Extras.txt].each do |list|
  next unless File.exist?(list)
  File.foreach(list) do |line|
    name = File.basename(line.strip, ".json")
    next if name.empty?
    add.call(
      id: "scoop-#{slug(name)}",
      name: "Scoop #{name} cache",
      description: "Windows app cache paths for Scoop package #{name} (ScoopInstaller Main/Extras bucket).",
      category: "app-cache",
      platform: "windows",
      paths: [
        "%LOCALAPPDATA%/#{name}/Cache",
        "%LOCALAPPDATA%/#{name}/GPUCache",
        "%LOCALAPPDATA%/#{name}/Code Cache",
        "%APPDATA%/#{name}/Cache",
        "%APPDATA%/#{name}/logs",
        "%USERPROFILE%/scoop/apps/#{name}/current/cache"
      ],
      min_age: 7
    )
  end
end

# Homebrew casks: Library Caches/Logs named after token / app
if File.exist?("/tmp/brew-cask.json")
  JSON.parse(File.read("/tmp/brew-cask.json")).each do |cask|
    token = cask["token"].to_s
    next if token.empty?
    names = Array(cask["name"]).map(&:to_s)
    apps = []
    Array(cask["artifacts"]).each do |art|
      next unless art.is_a?(Hash) && art["app"]
      Array(art["app"]).each { |a| apps << a.to_s.sub(/\.app$/i, "") }
    end
    label = (apps.first || names.first || token)
    add.call(
      id: "cask-cache-#{slug(token)}",
      name: "Cask cache #{token}",
      description: "Homebrew cask #{token} typical leftover caches/logs (#{label}). Not Application Support documents.",
      category: "app-cache",
      platform: "macos",
      paths: [
        "$HOME/Library/Caches/#{token}",
        "$HOME/Library/Caches/#{label}",
        "$HOME/Library/Logs/#{token}",
        "$HOME/Library/Logs/#{label}",
        "$HOME/Library/Saved Application State/#{token}.savedState"
      ].uniq,
      min_age: 7
    )
  end
end

if rules.size < TARGET
  warn "only #{rules.size} new rules (need #{TARGET})"
end

FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src5kb-*.yaml")).each { |f| File.delete(f) }
rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  path = File.join(OUT, format("src5kb-%02d.yaml", i))
  File.write(path, YAML.dump(
    "pack" => "src5kb-#{i}",
    "description" => "Second community import: Flathub + Homebrew casks + Scoop. Chunk #{i}.",
    "rules" => chunk
  ))
end

puts "wrote #{rules.size} rules (need>=#{TARGET}) macos=#{rules.count { |r| r['platform']=='macos' }} windows=#{rules.count { |r| r['platform']=='windows' }} linux=#{rules.count { |r| r['platform']=='linux' }}"
