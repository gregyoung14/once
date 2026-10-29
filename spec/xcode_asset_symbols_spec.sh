#shellcheck shell=bash

Describe 'Xcode asset symbol setting'
  BeforeEach 'setup_workspace'
  AfterEach 'cleanup_workspace'

  asset_toolchain_unavailable() {
    [ "$(uname -s)" != Darwin ] || ! xcrun --find actool >/dev/null 2>&1
  }

  exercise_asset_symbols() {
    cp -R "$REPO_ROOT/fixtures/xcode_asset_symbols/." "$WORKSPACE/"
    ONCE_CHANGE_TRACKER=0 once build Assets --format json > "$WORKSPACE/build.json" 2> "$WORKSPACE/build.err" || { cat "$WORKSPACE/build.err"; return 1; }
    [ -s "$WORKSPACE/.once/out/Assets/Assets.app/Contents/Resources/Assets.car" ] || return 1
    [ ! -e "$WORKSPACE/.once/out/Assets/GeneratedAssetSymbols.swift" ] || return 1
    [ "$("$WORKSPACE/.once/out/Assets/Assets.app/Contents/MacOS/Assets")" = 'asset color loaded' ] || return 1
    sed -i '' 's/ASSETCATALOG_COMPILER_GENERATE_ASSET_SYMBOLS = NO/ASSETCATALOG_COMPILER_GENERATE_ASSET_SYMBOLS = YES/' "$WORKSPACE/Assets.xcodeproj/project.pbxproj"
    sed -i '' '/extension NSColor {/,/^}/d' "$WORKSPACE/main.swift"
    ONCE_CHANGE_TRACKER=0 once build Assets --format json > "$WORKSPACE/build.json" 2> "$WORKSPACE/build.err" || { cat "$WORKSPACE/build.err"; return 1; }
    [ -s "$WORKSPACE/.once/out/Assets/GeneratedAssetSymbols.swift" ] || return 1
    [ "$("$WORKSPACE/.once/out/Assets/Assets.app/Contents/MacOS/Assets")" = 'asset color loaded' ] || return 1
  }

  It 'disables conflicting accessors without losing runtime assets, and enables them when requested'
    Skip if 'requires Xcode' asset_toolchain_unavailable
    When call exercise_asset_symbols
    The status should be success
    The output should equal ''
  End
End
