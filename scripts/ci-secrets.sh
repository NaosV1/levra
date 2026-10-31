#!/bin/sh
# Gives the GitHub workflows what they need to sign and notarize Levra, as
# secrets of the repository: the Developer ID certificate with its private
# key (a .p12 file, exported from Keychain Access) and the notarization
# credentials. Saves the latter on this Mac too, under the "levra" profile
# that scripts/release.sh uses. Passwords are asked without being shown, and
# Apple checks the credentials before anything is sent to GitHub.
#
#   scripts/ci-secrets.sh path/to/developer-id.p12
#
# P12_PASSWORD gives the .p12's password instead of asking for it.
set -eu

p12="${1:?usage : scripts/ci-secrets.sh certificat.p12}"
cd "$(dirname "$0")/.."
trap 'stty echo 2>/dev/null' EXIT

ask_secret() {
    printf '%s : ' "$1" >&2
    stty -echo
    read -r value
    stty echo
    printf '\n' >&2
    printf '%s' "$value"
}

P12_PASSWORD="${P12_PASSWORD:-$(ask_secret "Mot de passe du .p12")}"
export P12_PASSWORD
# The system's LibreSSL reads the .p12 files that Keychain Access exports.
subject=$(/usr/bin/openssl pkcs12 -in "$p12" -nokeys -passin env:P12_PASSWORD 2>/dev/null \
    | /usr/bin/openssl x509 -noout -subject 2>/dev/null) || subject=""
case "$subject" in
    *"Developer ID Application:"*) ;;
    *)
        echo "Ce .p12 ne contient pas de certificat « Developer ID Application », ou le mot de passe est faux." >&2
        exit 1
        ;;
esac
team=$(printf '%s' "$subject" | sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p')
echo "Équipe : $team"

printf 'Adresse du compte Apple Developer : ' >&2
read -r apple_id
app_password=$(ask_secret "Mot de passe d'app (account.apple.com)")

echo "→ Vérification auprès d'Apple, profil « levra » de ce Mac"
xcrun notarytool store-credentials levra \
    --apple-id "$apple_id" --team-id "$team" --password "$app_password"

echo "→ Secrets GitHub"
base64 -i "$p12" | gh secret set DEVELOPER_ID_P12
printf '%s' "$P12_PASSWORD" | gh secret set DEVELOPER_ID_P12_PASSWORD
printf '%s' "$apple_id" | gh secret set NOTARY_APPLE_ID
printf '%s' "$app_password" | gh secret set NOTARY_PASSWORD
printf '%s' "$team" | gh secret set NOTARY_TEAM_ID
echo "✓ Les builds de main et les tags v… seront signés et notarisés."
