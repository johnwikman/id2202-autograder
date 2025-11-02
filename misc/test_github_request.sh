#!/usr/bin/env bash

if [ $# != 1 ]; then
    echo "usage: $0 COMMIT_HASH"
    exit 1
fi

COMMIT_HASH="$1"
MESSAGE="#hello"

ORG="ID2202-jwikman-test"
REPO="id2202-testgrader"
SECRET_KEY="s3cr3t"

PAYLOAD=$(cat <<EOF
{
  "ref": "refs/heads/master",
  "repository": {
    "name": "$REPO",
    "full_name": "$ORG/$REPO",
    "organization": "$ORG"
  },
  "pusher": {
    "name": "test",
    "email": "test-id2202@kth.se"
  },
  "head_commit": {
    "id": "$COMMIT_HASH",
    "message": "$MESSAGE"
  }
}
EOF
)

#openssl dgst -sha256 -hmac "your_secret_key" -binary -out hash.bin input.txt
# example output from openssl:
# SHA2-256(stdin)= 03258a2f7d9d8a6e6ed40ee489e60cc6b1d01a14d8bb0e26fe5d03c1096648f9
HMAC_OUTPUT=$(printf "$PAYLOAD" | openssl dgst -sha256 -hmac "$SECRET_KEY" -hex | awk '{print $2;}')


echo "$PAYLOAD"

curl -L -v -X POST \
  -H "Content-Type: application/json" \
  -H "User-Agent: GitHub-Hookshot/f99c027" \
  -H "X-GitHub-Delivery: 9abd4920-92ff-11f0-9c7d-d21edefb7d88" \
  -H "X-GitHub-Enterprise-Host: gits-15.sys.kth.se" \
  -H "X-GitHub-Enterprise-Version: 3.16.5" \
  -H "X-GitHub-Event: push" \
  -H "X-GitHub-Hook-ID: 1665" \
  -H "X-GitHub-Hook-Installation-Target-ID: 26328" \
  -H "X-GitHub-Hook-Installation-Target-Type: organization" \
  -H "X-Hub-Signature-256: sha256=$HMAC_OUTPUT" \
 "http://127.0.0.1:8080/api/github-submit" \
 -d "$PAYLOAD"

echo
