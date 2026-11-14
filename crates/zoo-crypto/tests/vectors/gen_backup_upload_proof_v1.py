"""鍵バックアップのアップロードの証明（docs/e2ee.md の「アップロードの証明」）を、zoo-crypto とは別に計算する。

    nix develop -c python3 -I crates/zoo-crypto/tests/vectors/gen_backup_upload_proof_v1.py

出力は `crates/zoo-crypto/src/backup_proof.rs` の `VECTOR_PROOF`。バックアップ鍵の秘密鍵は 0x11 の 32 バイト、
サーバーの upload key の秘密鍵は 0x22 の 32 バイト。
"""

import base64
import struct

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

USER = "00000000-0000-4000-8000-000000000001"
VERSION = "12"


def lp(text):
    data = text.encode()
    return struct.pack(">I", len(data)) + data


backup = X25519PrivateKey.from_private_bytes(bytes([0x11] * 32))
server = X25519PrivateKey.from_private_bytes(bytes([0x22] * 32))
shared = backup.exchange(server.public_key())
assert shared == server.exchange(backup.public_key())
info = lp("zoo e2ee megolm backup upload v1") + lp(USER) + lp(VERSION)
proof = HKDF(algorithm=hashes.SHA256(), length=32, salt=bytes(32), info=info).derive(shared)
print(base64.b64encode(proof).decode().rstrip("="))
