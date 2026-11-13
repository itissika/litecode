#!/usr/bin/env python3
"""Start the one accounting exit for a campaign manifest.

The proxy container must write the campaign's exact `egress_log`; a proxy
writing somewhere else passes a TCP check but leaves the report with zero
rows. This helper derives the bind mount, file name and port from the same
campaign manifest the runner and collector use.

    python3 eval/harbor/bench5/start_proxy.py \
      --campaign eval/harbor/bench5/campaigns/bench5.yaml

The key is mounted read-only and never passed as an argument or printed.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]

try:
    from campaign import CampaignError, load_campaign
except ImportError:
    from .campaign import CampaignError, load_campaign

CONTAINER = "litecode-egress"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--campaign", default=str(HERE / "campaigns/bench5.yaml")
    )
    parser.add_argument(
        "--key-file", default=str(Path.home() / "eval/deepseek_key")
    )
    parser.add_argument("--container", default=CONTAINER)
    args = parser.parse_args(argv)
    try:
        campaign = load_campaign(args.campaign)
        parsed = urlsplit(campaign.egress_root)
        port = parsed.port or 8399
        if parsed.hostname not in ("172.17.0.1", "host.docker.internal", "0.0.0.0"):
            raise CampaignError(
                "campaign.egress_root must be the docker-bridge address, got "
                + campaign.egress_root
            )
        key = Path(args.key_file).expanduser().resolve()
        if not key.is_file():
            raise CampaignError(f"key file not found: {key}")
        proxy_dir = REPO / "eval/proxy"
        if not (proxy_dir / "deepseek_proxy.py").is_file():
            raise CampaignError(f"proxy code not found under {proxy_dir}")
        log = campaign.egress_log
        log.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["docker", "rm", "-f", args.container], capture_output=True)
        command = [
            "docker",
            "run",
            "-d",
            "--name",
            args.container,
            "--restart",
            "unless-stopped",
            "-p",
            f"{port}:{port}",
            "-v",
            f"{proxy_dir}:/p:ro",
            "-v",
            f"{key}:/k/deepseek_key:ro",
            "-v",
            f"{log.parent}:/logs",
            "python:3.11",
            "python",
            "/p/deepseek_proxy.py",
            "--host",
            "0.0.0.0",
            "--port",
            str(port),
            "--key-file",
            "/k/deepseek_key",
            "--log",
            f"/logs/{log.name}",
        ]
        result = subprocess.run(command, capture_output=True, text=True)
        if result.returncode != 0:
            raise CampaignError(result.stderr.strip() or "docker run failed")
        health = f"http://172.17.0.1:{port}/healthz"
        for _ in range(40):
            try:
                with urllib.request.urlopen(health, timeout=1) as response:
                    payload = json.loads(response.read().decode("utf-8"))
                    if payload.get("ok"):
                        print(
                            f"proxy {args.container} -> {campaign.egress_root} "
                            f"log={log}",
                            flush=True,
                        )
                        return 0
            except OSError:
                time.sleep(0.25)
        raise CampaignError(f"proxy started but {health} is not healthy")
    except (OSError, ValueError, CampaignError, subprocess.SubprocessError) as error:
        print(f"start_proxy: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())