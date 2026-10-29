def _apple_export_artifact(ctx, provider, info_plist, minimum_os, architecture):
    platform = "macos" if provider["platform"] in ["macos", "macosx"] else provider["platform"]
    platform = "visionos" if platform == "xros" else platform
    sdk_variant = "device" if platform == "macos" else provider["sdk_variant"]
    archive = declare_output("artifact/" + provider["product_name"] + ".zip")
    write_archive(
        [_artifact_entry(provider["app_path"], provider["product_name"] + ".app", kind = "tree", mode = 493)],
        archive,
        format = "zip",
        inputs = provider["app_files"],
        identifier = ctx["label"]["id"] + ":artifact-package",
    )
    interfaces = {
        "dev.once.apple-install.v1": {
            "payload": "bundle",
            "spec": {"platform": platform, "sdk_variant": sdk_variant, "architecture": architecture, "minimum_os": minimum_os, "bundle_identifier": provider["bundle_id"]},
        },
    }
    payloads = {"bundle": {"path": archive, "media_type": "application/zip"}}
    if host_os() == "macos":
        metadata = declare_output("artifact/metadata.json")
        expand_actions(
            implementation = "apple_artifact_metadata_plan",
            inputs = [provider["app_executable"], info_plist],
            outputs = [metadata],
            args = {"executable": provider["app_executable"], "info_plist": info_plist, "metadata": metadata, "platform": platform, "sdk_variant": sdk_variant, "developer_dir": provider["xcode_developer_dir"]},
        )
        payloads["metadata"] = {"path": metadata, "media_type": "application/json"}
        interfaces["dev.tuist.preview.v1"] = {"payload": "bundle", "spec": {"metadata_payload": "metadata"}}
    provider["artifact_info"] = _artifact_info([{
        "name": "product",
        "payloads": payloads,
        "interfaces": interfaces,
        "selectors": {"platform": platform, "sdk_variant": sdk_variant, "architecture": architecture},
    }])
    return provider

def apple_artifact_metadata_plan(ctx):
    spec = ctx["args"]
    root = workspace_root() + "/"
    plist = json_decode(host_command([host_which("plutil"), "-convert", "json", "-o", "-", root + spec["info_plist"]]))
    developer = spec["developer_dir"]
    env = {"DEVELOPER_DIR": developer} if developer else {}
    uuid_output = host_command([host_which("xcrun"), "dwarfdump", "--uuid", root + spec["executable"]], env = env)
    uuids = [line.split()[1] for line in uuid_output.split("\n") if line.startswith("UUID: ")]
    if not uuids:
        _diagnostic_fail("artifact_binary_identity_missing", None, "The application has no Mach-O UUID", "Build the application with a linker that emits UUIDs")
    platform = spec["platform"]
    supported_platform = platform + "_simulator" if platform != "macos" and spec["sdk_variant"] == "simulator" else platform
    write_path(spec["metadata"], _json_encode({
        "type": "app_bundle",
        "display_name": plist.get("CFBundleDisplayName") or plist.get("CFBundleName") or "",
        "bundle_identifier": plist.get("CFBundleIdentifier") or "",
        "version": plist.get("CFBundleShortVersionString") or "",
        "build_version": plist.get("CFBundleVersion") or "",
        "binary_id": uuids[0],
        "supported_platforms": [supported_platform],
    }))
