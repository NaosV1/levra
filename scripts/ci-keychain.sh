#!/bin/sh
# On a GitHub runner, before scripts/release.sh: puts the Developer ID
# certificate in a keychain of its own, made the default one, so that
# codesign finds it and notarytool finds the credentials saved next to it
# under the "levra" profile. Reads the secrets that scripts/ci-secrets.sh
# sets. The runner is thrown away after the job, keychain included.
set -eu

for secret in DEVELOPER_ID_P12 DEVELOPER_ID_P12_PASSWORD NOTARY_APPLE_ID NOTARY_PASSWORD NOTARY_TEAM_ID; do
    if [ -z "$(printenv "$secret")" ]; then
        echo "::error::Secret $secret manquant : voir « Distribution » dans README.md."
        exit 1
    fi
done

keychain="$RUNNER_TEMP/levra.keychain-db"
password=$(openssl rand -base64 24)
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
printf '%s' "$DEVELOPER_ID_P12" | base64 --decode > "$RUNNER_TEMP/developer-id.p12"
security import "$RUNNER_TEMP/developer-id.p12" -k "$keychain" \
    -P "$DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign
rm "$RUNNER_TEMP/developer-id.p12"
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain"
security default-keychain -d user -s "$keychain"
xcrun notarytool store-credentials levra \
    --apple-id "$NOTARY_APPLE_ID" --team-id "$NOTARY_TEAM_ID" --password "$NOTARY_PASSWORD"
