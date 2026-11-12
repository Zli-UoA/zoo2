"""secret_storage_v1.json を作る（zoo-crypto とは別の実装で計算し直すため）。

docs/e2ee.md の「SSSS（秘密の暗号化）」「鍵スロット」「リカバリーキーの形式」「鍵バックアップ」の
手順を、Python の cryptography でそのまま書いたもの。乱数の代わりに決まったバイト列を使う。

    nix develop -c python3 -I crates/zoo-crypto/tests/vectors/gen_secret_storage_v1.py \
      > crates/zoo-crypto/tests/vectors/secret_storage_v1.json

Python と cryptography は flake.nix の開発シェルに入れてある。
"""

import base64
import hashlib
import hmac
import json
import struct

from cryptography.hazmat.primitives import hashes, padding
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


def seq(start, n):
    return bytes((start + i) & 0xFF for i in range(n))


def b64(data):
    return base64.b64encode(data).decode().rstrip("=")


def hkdf(ikm, salt, info, length):
    return HKDF(algorithm=hashes.SHA256(), length=length, salt=salt, info=info).derive(ikm)


def lp(*parts):
    out = b""
    for part in parts:
        raw = part.encode()
        out += struct.pack(">I", len(raw)) + raw
    return out


def aes_ctr(key, iv, data):
    enc = Cipher(algorithms.AES(key), modes.CTR(iv)).encryptor()
    return enc.update(data) + enc.finalize()


def ssss_encrypt(key, name, iv, plaintext):
    okm = hkdf(key, b"\0" * 32, name.encode(), 64)
    ciphertext = aes_ctr(okm[:32], iv, plaintext)
    mac = hmac.new(okm[32:], ciphertext, hashlib.sha256).digest()
    return ciphertext, mac


ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def base58(data):
    n = int.from_bytes(data, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = ALPHABET[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def recovery_display(key):
    raw = b"\x8b\x01" + key
    parity = 0
    for b in raw:
        parity ^= b
    encoded = base58(raw + bytes([parity]))
    return " ".join(encoded[i : i + 4] for i in range(0, len(encoded), 4))


USER_ID = "00000000-0000-0000-0000-00000000000a"
KEY_ID = "5d6a1d1c-0c4f-4c3a-9b1e-2f8a7e6d5c4b"
ssss_key = seq(0x00, 32)

check_iv = bytearray(seq(0x10, 16))
check_iv[8] &= 0x7F
check_iv = bytes(check_iv)
_, check_mac = ssss_encrypt(ssss_key, "", check_iv, b"\0" * 32)

secret_name = "m.cross_signing.master"
secret_plaintext = b64(seq(0x20, 32))
secret_iv = bytearray(seq(0x30, 16))
secret_iv[8] &= 0x7F
secret_iv = bytes(secret_iv)
secret_ct, secret_mac = ssss_encrypt(ssss_key, secret_name, secret_iv, secret_plaintext.encode())


def slot(kind, slot_id, ikm, info, nonce):
    kek = hkdf(ikm, None, info, 32)
    aad = lp("zoo e2ee ssss slot v1", kind, USER_ID, KEY_ID, slot_id)
    ciphertext = AESGCM(kek).encrypt(nonce, ssss_key, aad)
    return kek, aad, ciphertext


passkey_slot_id = "7f1e2d3c-4b5a-4968-8776-655443322110"
prf_salt = seq(0x40, 32)
prf_output = seq(0x60, 32)
passkey_nonce = seq(0x80, 12)
pk_kek, pk_aad, pk_ct = slot("passkey", passkey_slot_id, prf_output, b"zoo e2ee ssss v1", passkey_nonce)

recovery_slot_id = "0a1b2c3d-4e5f-4a6b-8c7d-8e9fa0b1c2d3"
recovery_key = seq(0xA0, 32)
recovery_nonce = seq(0x90, 12)
rc_kek, rc_aad, rc_ct = slot(
    "recovery", recovery_slot_id, recovery_key, b"zoo e2ee ssss recovery v1", recovery_nonce
)

backup_private = seq(0xC0, 32)
ephemeral_private = seq(0xE0, 32)
backup_public = (
    X25519PrivateKey.from_private_bytes(backup_private)
    .public_key()
    .public_bytes(Encoding.Raw, PublicFormat.Raw)
)
ephemeral_key = X25519PrivateKey.from_private_bytes(ephemeral_private)
ephemeral_public = ephemeral_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
shared = ephemeral_key.exchange(
    X25519PrivateKey.from_private_bytes(backup_private).public_key()
)
okm = hkdf(shared, b"\0" * 32, b"zoo e2ee megolm backup v1", 80)
plaintext = json.dumps(
    {
        "algorithm": "m.megolm.v1.aes-sha2",
        "channel_id": "c0000000-0000-4000-8000-000000000001",
        "epoch": 3,
        "session_id": "sessionid",
        "session_key": "exportedsessionkey",
        "sender_key": b64(seq(0x01, 32)),
        "sender_claimed_keys": {"ed25519": b64(seq(0x02, 32))},
        "sender_user_id": "00000000-0000-0000-0000-00000000000b",
        "sender_device_id": "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718",
    },
    separators=(",", ":"),
)
padder = padding.PKCS7(128).padder()
padded = padder.update(plaintext.encode()) + padder.finalize()
enc = Cipher(algorithms.AES(okm[:32]), modes.CBC(okm[64:80])).encryptor()
backup_ct = enc.update(padded) + enc.finalize()
backup_mac = hmac.new(okm[32:64], backup_ct, hashlib.sha256).digest()

print(
    json.dumps(
        {
            "generated_by": "tests/vectors/gen_secret_storage_v1.py（Python の cryptography。手順はファイルの先頭）",
            "user_id": USER_ID,
            "ssss": {
                "key_id": KEY_ID,
                "key": ssss_key.hex(),
                "check_iv": check_iv.hex(),
                "check_mac": b64(check_mac),
                "secret_name": secret_name,
                "secret_plaintext": secret_plaintext,
                "secret_iv": secret_iv.hex(),
                "secret_ciphertext": b64(secret_ct),
                "secret_mac": b64(secret_mac),
            },
            "passkey_slot": {
                "slot_id": passkey_slot_id,
                "credential_id": "AQIDBAUGBwg",
                "prf_salt": prf_salt.hex(),
                "prf_output": prf_output.hex(),
                "nonce": passkey_nonce.hex(),
                "aad": pk_aad.hex(),
                "kek": pk_kek.hex(),
                "ciphertext": b64(pk_ct),
            },
            "recovery_slot": {
                "slot_id": recovery_slot_id,
                "recovery_key": recovery_key.hex(),
                "recovery_key_display": recovery_display(recovery_key),
                "nonce": recovery_nonce.hex(),
                "aad": rc_aad.hex(),
                "kek": rc_kek.hex(),
                "ciphertext": b64(rc_ct),
            },
            "backup": {
                "private_key": backup_private.hex(),
                "public_key": b64(backup_public),
                "ephemeral_private_key": ephemeral_private.hex(),
                "plaintext": plaintext,
                "ephemeral": b64(ephemeral_public),
                "ciphertext": b64(backup_ct),
                "mac": b64(backup_mac),
            },
        },
        ensure_ascii=False,
        indent=2,
    )
)
