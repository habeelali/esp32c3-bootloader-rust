#!/usr/bin/env python3
"""Push OTA firmware to a running QEMU ESP32-C3 via TCP UART.

Usage:
    python3 ota_push.py --port 5555 --firmware test_app_v2.img

The QEMU instance must be started with:
    -serial tcp::5555,server,nowait
"""

import argparse
import os
import socket
import sys
import time

CHUNK = 256
TIMEOUT = 30.0   # seconds to wait for each response


def recv_until(sock: socket.socket, marker: bytes, timeout: float = TIMEOUT) -> bytes:
    buf = b""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            sock.settimeout(0.1)
            data = sock.recv(4096)
            if data:
                buf += data
                if marker in buf:
                    return buf
        except socket.timeout:
            pass
    raise TimeoutError(f"timed out waiting for {marker!r}, got: {buf!r}")


def main():
    ap = argparse.ArgumentParser(description="Push OTA firmware to QEMU ESP32-C3")
    ap.add_argument("--port",     type=int, default=5555, help="TCP port (default 5555)")
    ap.add_argument("--host",     default="localhost",    help="QEMU host (default localhost)")
    ap.add_argument("--firmware", required=True,          help="Firmware image to push")
    args = ap.parse_args()

    if not os.path.exists(args.firmware):
        print(f"ERROR: firmware file not found: {args.firmware}", file=sys.stderr)
        sys.exit(1)

    with open(args.firmware, "rb") as f:
        firmware = f.read()

    size = len(firmware)
    print(f"Firmware: {args.firmware}  ({size} bytes)")

    print(f"Connecting to {args.host}:{args.port}...")
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    for attempt in range(20):
        try:
            sock.connect((args.host, args.port))
            break
        except ConnectionRefusedError:
            if attempt == 19:
                print("ERROR: could not connect — is QEMU running with TCP serial?",
                      file=sys.stderr)
                sys.exit(1)
            time.sleep(0.5)
    print("Connected.")

    # Wait for the shell prompt (may take ~10s for QEMU boot + 5s countdown)
    print("Waiting for shell prompt (up to 20s)...")
    try:
        recv_until(sock, b"esp32c3> ", timeout=20.0)
    except TimeoutError:
        # Send a newline to refresh the prompt in case we connected mid-session
        sock.sendall(b"\r\n")
        recv_until(sock, b"esp32c3> ", timeout=5.0)
    print("Shell ready.")

    # Send OTA command
    cmd = f"ota {size}\r\n".encode()
    print(f"Sending: ota {size}")
    sock.sendall(cmd)

    # Wait for READY
    recv_until(sock, b"READY", timeout=10.0)
    print("Device ready. Streaming firmware...")

    # Stream firmware in chunks, collect '.' ACKs
    offset = 0
    chunk_num = 0
    total_chunks = (size + CHUNK - 1) // CHUNK

    while offset < size:
        chunk = firmware[offset: offset + CHUNK]
        sock.sendall(chunk)
        offset += len(chunk)
        chunk_num += 1

        # Wait for '.' ACK
        recv_until(sock, b".", timeout=10.0)
        pct = offset * 100 // size
        print(f"  {chunk_num}/{total_chunks}  {offset}/{size} bytes  ({pct}%)",
              end="\r", flush=True)

    print()

    # Wait for OK
    print("Waiting for completion...")
    recv_until(sock, b"OK", timeout=10.0)
    print("OTA write complete. Device is resetting...")

    sock.close()
    print("\nDone. QEMU will reboot and boot from ota_0 (v2).")
    print("Reconnect to port 5555 (or restart QEMU with mon:stdio) to see v2 output.")


if __name__ == "__main__":
    main()
