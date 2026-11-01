"""Generate the public main-channel manifest after building a signed APK."""
import hashlib
import json
import re
import sys
from pathlib import Path


def manifest(apk: Path, version_code: int, version_name: str, commit: str) -> dict:
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("Expected a full Git commit SHA")
    if not 0 < version_code <= 2**31 - 1:
        raise ValueError("Invalid Android version code")
    size = apk.stat().st_size
    if not 0 < size <= 200 * 1024 * 1024:
        raise ValueError("Invalid APK size")
    with apk.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    return {
        "channel": "main",
        "packageName": "works.zli.zoo",
        "versionCode": version_code,
        "versionName": version_name,
        "commit": commit,
        "url": f"https://github.com/Zli-UoA/zoo2/releases/download/android-main/zoo-android-main-{commit}.apk",
        "size": size,
        "sha256": checksum,
    }


if __name__ == "__main__":
    apk, version_code, version_name, commit, output = sys.argv[1:]
    Path(output).write_text(json.dumps(manifest(Path(apk), int(version_code), version_name, commit), ensure_ascii=False, indent=2) + "\n")
