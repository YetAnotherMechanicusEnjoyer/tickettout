#!/usr/bin/env python3
import argparse
import hashlib
import hmac
import json
import sys
import urllib.error
import urllib.request


def compute_hash(record: dict) -> str:
    payload = record.get("payload")
    payload_str = (
        json.dumps(payload, separators=(",", ":"), sort_keys=True, ensure_ascii=False)
        if payload is not None
        else ""
    )

    fields = [
        str(record.get("id") or ""),
        str(record.get("occurred_at", "")),
        str(record.get("actor_id") or ""),
        str(record.get("actor_role") or ""),
        str(record.get("action") or ""),
        str(record.get("target_type") or ""),
        str(record.get("target_id") or ""),
        payload_str,
        str(record.get("ip") or ""),
        str(record.get("previous_hash") or ""),
    ]
    raw = "|".join(fields)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def verify_chain(records: list) -> tuple[bool, dict | None, str]:
    expected_prev = "GENESIS"
    for record in records:
        stored_prev = record.get("previous_hash")
        if stored_prev != expected_prev:
            reason = (
                f"previous_hash stocké = {stored_prev!r}, "
                f"attendu (hash du précédent enregistrement intact) = {expected_prev!r}"
            )
            return False, record, reason
        expected_prev = compute_hash(record)
    return True, None, ""


def fetch_all_via_api(base_url: str, token: str) -> list:
    records = []
    page = 1
    per_page = 200
    while True:
        url = f"{base_url}/api/v1/admin/audit?page={page}&per_page={per_page}"
        req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
        try:
            with urllib.request.urlopen(req) as resp:
                body = json.loads(resp.read())
        except urllib.error.HTTPError as e:
            print(f"Erreur HTTP {e.code} en interrogeant {url}: {e.read().decode(errors='replace')}", file=sys.stderr)
            sys.exit(2)
        records.extend(body["items"])
        if page >= body["total_pages"] or not body["items"]:
            break
        page += 1
    records.sort(key=lambda r: (r["occurred_at"], r["id"]))
    return records


def load_export_file(path: str, key: str) -> list:
    with open(path, "r", encoding="utf-8") as f:
        export = json.load(f)

    data_json = export["data_json"]
    signature = export["signature"]

    computed_hmac = hmac.new(key.encode("utf-8"), data_json.encode("utf-8"), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(computed_hmac, signature):
        print("NON CONFORME")
        print("La signature HMAC de l'export ne correspond pas à son contenu.")
        print("Soit la clé est incorrecte, soit le fichier d'export a été modifié après signature.")
        sys.exit(1)

    records = json.loads(data_json)
    records.sort(key=lambda r: (r["occurred_at"], r["id"]))
    return records


def main() -> None:
    parser = argparse.ArgumentParser(description="Vérifie l'intégrité de la chaîne d'audit CartePro.")
    parser.add_argument("--api", help="URL de base de l'API, ex: http://localhost:8080")
    parser.add_argument("--token", help="UUID de l'admin (utilisé comme Bearer token)")
    parser.add_argument("--export-file", help="Fichier JSON d'export (GET /v1/admin/audit/export)")
    parser.add_argument("--key", help="Clé HMAC (AUDIT_EXPORT_KEY) pour vérifier l'export")
    args = parser.parse_args()

    if args.api and args.token:
        records = fetch_all_via_api(args.api, args.token)
        source = f"API {args.api} ({len(records)} enregistrement(s))"
    elif args.export_file and args.key:
        records = load_export_file(args.export_file, args.key)
        source = f"export {args.export_file}, signature HMAC vérifiée ({len(records)} enregistrement(s))"
    else:
        parser.error("fournir soit --api + --token, soit --export-file + --key")
        return

    if not records:
        print("CONFORME")
        print(f"Source : {source}. Chaîne vide, rien à vérifier.")
        sys.exit(0)

    ok, broken, reason = verify_chain(records)

    if ok:
        print("CONFORME")
        print(f"Source : {source}.")
        print(f"Chaîne intacte du premier au dernier enregistrement (id={records[0]['id']} -> id={records[-1]['id']}).")
        sys.exit(0)
    else:
        print("NON CONFORME")
        print(f"Source : {source}.")
        print("La chaîne rompt au premier enregistrement suivant :")
        print(f"  id           : {broken['id']}")
        print(f"  occurred_at  : {broken['occurred_at']}")
        print(f"  action       : {broken['action']}")
        print(f"  actor_id     : {broken.get('actor_id')}")
        print(f"  target_type  : {broken.get('target_type')}")
        print(f"  target_id    : {broken.get('target_id')}")
        print(f"Raison : {reason}")
        sys.exit(1)


if __name__ == "__main__":
    main()
