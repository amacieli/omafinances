# OmaFinances Privacy Policy

**Effective date:** November 21, 2026
**Contact:** adam.macielinski@gmail.com

OmaFinances is a personal finance application that runs entirely on the computer of the person who installs it. This policy explains what data it touches and where that data goes.

## Summary

- OmaFinances has **no server**. The developer does not receive, collect, store, or have access to your data.
- Your financial data is stored **only on your own computer**, in an encrypted local database.
- The app's Google access is limited to **sending** email from your own Gmail account. It cannot read, list, or delete your mail.

## Data the app handles

| Data | Source | Where it is stored |
|---|---|---|
| Accounts, balances, transactions, categories | Your Lunch Money account, via the Lunch Money API token you provide | Encrypted (SQLCipher) database on your computer |
| Notes and nicknames you enter | You | The same local database |
| Lunch Money API token, database passphrase | You | Your operating system's keyring or a local file readable only by you |
| Google refresh token | Google, after you approve access | Your operating system's keyring |

## Google user data

If you choose to enable emailed reports, OmaFinances asks Google for two permissions:

- `https://www.googleapis.com/auth/gmail.send`: send email on your behalf.
- `https://www.googleapis.com/auth/userinfo.email`: learn the address of the account you authorized.

How this access is used:

- The app uses it only to send the reports you configure, to recipients you have allowed in your settings.
- The app does not read, search, or modify your Gmail messages, contacts, or any other Google data.
- The authorization token stays on your computer. It is never sent to the developer or any third party other than Google.
- You can revoke access at any time at https://myaccount.google.com/permissions and delete the stored token from your keyring.

OmaFinances' use and transfer of information received from Google APIs adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements.

## Sharing

The developer shares nothing, because the developer receives nothing. The app talks directly from your computer to:

- **Lunch Money** (to import your financial data, read-only), under Lunch Money's own terms and privacy policy.
- **Google** (only to send reports you enabled).

Report emails contain financial figures and are delivered through Gmail to the recipients you specify. Choose recipients accordingly.

## No advertising, no analytics, no sale of data

The app contains no ads, no tracking, and no telemetry, and your data is never sold or used to train AI or machine-learning models.

## Retention and deletion

Data stays on your computer until you delete it. Deleting the database file, the application settings directory, and the keyring entries (`omafinances`) removes everything the app stores.

## Children

The app is not directed at children.

## Changes

If this policy changes, the updated version will be published at this address with a new effective date.

## Contact

Questions: adam.macielinski@gmail.com
