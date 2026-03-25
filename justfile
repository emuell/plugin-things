vst3_path := if os() == "windows" { "C:/Program\\ Files/Common\\ Files/VST3/Dev/" } else if os() == "macos" { "~/Library/Audio/Plug-Ins/VST3/" } else { error("Unexpected OS {{ os() }}") }
clap_path := if os() == "windows" { "C:/Program\\ Files/Common\\ Files/CLAP/Dev/" } else if os() == "macos" { "~/Library/Audio/Plug-Ins/CLAP/" } else { error("Unexpected OS {{ os() }}") }

default:
    @just --choose

[arg("target", pattern="debug|release")]
bundle-gain-plugin target="debug":
    cargo xtask-{{ target }} bundle gain-plugin {{ if target == "debug" { "" } else { "--release" } }}
    cp -rf ./target/bundled/gain-plugin.vst3 {{ vst3_path }}
    cp -rf ./target/bundled/gain-plugin.clap {{ clap_path }}
