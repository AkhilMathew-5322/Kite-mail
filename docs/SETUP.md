# Account & service setup

Step-by-step setup for each kind of account and service. Back to the [README](../README.md).

## Microsoft 365 (one time, done by a tenant admin)

1. **Entra admin center** → **App registrations → New registration**.
   - Name: `Kite Mail`
   - Supported account types: **this organizational directory only**
   - Redirect URI: **Public client/native (mobile & desktop)** → `http://localhost`
2. **API permissions → Add → APIs my organization uses → Office 365 Exchange Online → Delegated**, then tick `IMAP.AccessAsUser.All` and `SMTP.Send`. Also add **Microsoft Graph → Delegated** `offline_access`, `openid` and `email`. Then click **Grant admin consent**.
3. Copy the **Application (client) ID** and the **Directory (tenant) ID**.
4. Make sure each mailbox allows **IMAP** and **Authenticated SMTP**: in the Admin center go to **Users → Mail → Manage email apps**, or use PowerShell:
   ```powershell
   Set-CASMailbox -Identity user@yourdomain.com -ImapEnabled $true -SmtpClientAuthenticationDisabled $false
   ```
5. In Kite Mail, go to **Add account → Microsoft 365** and enter your email, the Client ID, and the Tenant (the tenant ID or your domain). Sign in when the browser opens.

If a sign-in expires, an ⚠ and a **Reconnect** button appear next to the account.

## Gmail (one time)
1. In https://console.cloud.google.com, create a project and enable the **Gmail API**.
2. On the **OAuth consent screen**, choose **Internal** (Workspace) or **External**, add the scope `https://mail.google.com/`, and add yourself under **Test users**.
3. Go to **Credentials → OAuth client ID → Desktop app** and copy the Client ID and Client secret.
4. In Kite Mail, go to **Add account → Gmail**.

> If the consent screen is **External** and still in **Testing**, Google ends the sign-in after 7 days. Use **Internal**, or click **Publish app**, to avoid that.
> Alternatively, use **Other (IMAP)** with `imap.gmail.com:993` / `smtp.gmail.com:465` and an App password.

## Other providers (IMAP)
**Add account → Other (IMAP)**. For example, Hostinger: `imap.hostinger.com:993` and `smtp.hostinger.com:465` with SSL/TLS.

## Nextcloud (contacts, tasks, backups)
1. In Nextcloud, go to **Settings → Security → Devices & sessions → Create new app password**, name it "Kite Mail", and copy the password.
2. In Kite Mail, go to **Settings → Nextcloud** and enter your server address (for example `https://cloud.yourcompany.com`), username and app password, then click **Connect**. The first sync starts immediately.
3. Optional: choose a task list, set the backup schedule and folder, and set the size limit for large attachments.

Contacts and tasks sync automatically every 15 minutes and a few seconds after each change. Use **Sync now** for an immediate sync.
If you edit the same contact on two devices at once, the server's version wins. Deletions sync in both directions.

## S3 storage (backups and large attachments)
Settings → **Cloud & Backup → S3**. Works with Amazon S3, Oracle Cloud Object Storage, Cloudflare R2, Wasabi, MinIO and other S3-compatible storage: enter the endpoint, region, bucket and an access key, then **Test**. Choose daily/weekly backups ("keep the latest N") and whether large attachments are sent as expiring download links.

## Calling with 3CX
The 3CX desktop and mobile apps already handle `tel:` links, so clicking a number dials through your own extension. A direct 3CX Call Control API integration would need extra licensing, PBX configuration and a stored API key, and you wouldn't gain anything for click-to-call. If you use the 3CX Web Client instead of the app, choose **Settings → Calling → 3CX Web Client** and enter `https://YOURPBX/webclient/#/call?phone={number}`.

## Grammar checking (LanguageTool)
Grammarly closed its SDK for third-party apps in 2024, so it can't be built in. LanguageTool is the open-source alternative. You can use the free public server, which is limited to roughly 20 checks per minute, or run your own:
```bash
docker run -d -p 8010:8010 erikvl87/languagetool     # then set http://SERVER:8010 in Settings → Writing
```
(A Nextcloud LanguageTool app also exists.)
