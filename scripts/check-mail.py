#!/usr/bin/env python3
"""Maintainer triage for contact@alix.study (forwarded to a web.de inbox).

Reads IMAP credentials from ~/.config/alix-mail/credentials (never from the
repo), fetches headers of recent messages addressed to the contact alias, and
classifies each as SPAM, MARKETING, or USEFUL by header evidence. Read-only:
nothing is moved, flagged, or deleted."""

import email
import email.utils
import imaplib
import os
import re
import sys
from email.header import decode_header, make_header

HOST = "imap.web.de"
ALIAS = "contact@alix.study"
CREDENTIALS = os.path.expanduser("~/.config/alix-mail/credentials")
RECENT = 200

SPAM_SUBJECT = re.compile(
    r"(you (have )?won|lottery|inheritance|prince|bitcoin.*(double|profit)"
    r"|urgent.*(transfer|payment)|account.*(suspend|verif)|\bviagra\b)",
    re.IGNORECASE,
)


def credentials():
    if not os.path.exists(CREDENTIALS):
        sys.exit(
            f"no credentials at {CREDENTIALS}\n"
            "create it (mode 600) with two lines:\n"
            "  user=<web.de address>\n"
            "  pass=<web.de app password (IMAP must be enabled in web.de settings)>"
        )
    values = {}
    with open(CREDENTIALS, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if "=" in line and not line.startswith("#"):
                key, _, value = line.partition("=")
                values[key.strip()] = value.strip()
    user, password = values.get("user"), values.get("pass")
    if not user or not password:
        sys.exit(f"{CREDENTIALS} must define user= and pass=")
    return user, password


def decoded(value):
    if value is None:
        return ""
    try:
        return str(make_header(decode_header(value)))
    except (UnicodeError, LookupError, ValueError):
        return value


def to_alias(message):
    fields = [
        message.get("To", ""),
        message.get("Cc", ""),
        message.get("Delivered-To", ""),
        message.get("X-Forwarded-For", ""),
        message.get("X-Original-To", ""),
        message.get("Envelope-To", ""),
    ]
    return any(ALIAS in field.lower() for field in fields)


def classify(message):
    auth = message.get("Authentication-Results", "").lower()
    subject = decoded(message.get("Subject", ""))
    if "spf=fail" in auth or "dkim=fail" in auth or SPAM_SUBJECT.search(subject):
        return "SPAM"
    if (
        message.get("List-Unsubscribe")
        or message.get("Precedence", "").lower() in ("bulk", "list")
        or message.get("X-Mailer-LID")
    ):
        return "MARKETING"
    return "USEFUL"


def main():
    user, password = credentials()
    imap = imaplib.IMAP4_SSL(HOST)
    try:
        imap.login(user, password)
        imap.select("INBOX", readonly=True)
        status, data = imap.search(None, "ALL")
        if status != "OK":
            sys.exit("IMAP search failed")
        ids = data[0].split()[-RECENT:]
        buckets = {"SPAM": [], "MARKETING": [], "USEFUL": []}
        for msg_id in ids:
            status, fetched = imap.fetch(
                msg_id, "(BODY.PEEK[HEADER])"
            )
            if status != "OK" or not fetched or fetched[0] is None:
                continue
            message = email.message_from_bytes(fetched[0][1])
            if not to_alias(message):
                continue
            sender = decoded(message.get("From", "?"))
            subject = decoded(message.get("Subject", "(no subject)"))
            date = email.utils.parsedate_to_datetime(
                message.get("Date")
            ).strftime("%Y-%m-%d") if message.get("Date") else "????-??-??"
            buckets[classify(message)].append(f"{date}  {sender}  |  {subject}")
        for name in ("USEFUL", "MARKETING", "SPAM"):
            rows = buckets[name]
            print(f"\n== {name} ({len(rows)}) ==")
            for row in rows:
                print(f"  {row}")
        total = sum(len(rows) for rows in buckets.values())
        print(
            f"\n{total} message(s) to {ALIAS} among the last {len(ids)} "
            "inbox messages. Classification is header-evidence only; read "
            "anything surprising yourself before acting on it."
        )
    finally:
        try:
            imap.logout()
        except (imaplib.IMAP4.error, OSError):
            pass


if __name__ == "__main__":
    main()
