# Signing in

Every page and API route except the health check (`/api/v1/health`) needs a signed-in account. A fresh install opens on a welcome page
where the first visitor creates the admin account and the first household; after that, admins add people from
**Account → People**, or from the server with `mise user`.

## Accounts and households

A household shares plans, favourites and lists; every account belongs to one. Admins can move people between
households and add new ones.

## Passwords

Passwords are stored as argon2id hashes. After five wrong attempts an account is locked for 30 seconds, doubling
with each further miss up to 15 minutes; the right password clears it. Changing a password signs out every other
browser. Locked out entirely? On the server:

```bash
echo 'a new password' | mise user set-password <username>
mise user admin <username>          # make an account an admin again
mise user list
```

## Single sign-on (OIDC)

Any OpenID Connect provider works: Authentik, Keycloak, Nextcloud (with the OIDC app), Google, and so on. Create a
confidential client in the provider with this redirect URI:

```
<PUBLIC_BASE_URL or http://<mise host>:8080>/api/auth/oidc/callback
```

Then set `AUTH_MODES=local,oidc` (or just `oidc`) and the `OIDC_*` settings in `.env.example`. The issuer is the URL
whose `/.well-known/openid-configuration` the provider serves (in Authentik: the application's
*OpenID Configuration Issuer*).

- A returning person is recognised by their provider identity. The first time, they are linked to an existing
  account with the same email if the provider says the email is verified; otherwise a new account is created,
  in a household of its own, or in `OIDC_HOUSEHOLD` when set.
- With `OIDC_ADMIN_GROUP`, admin rights follow membership of that group (the `groups` claim; in Authentik and
  Keycloak add a groups mapper or scope) at every sign-in.
- The flow uses PKCE and a nonce; sign-in attempts expire after 15 minutes and can be used once.

## API tokens

**Account → API tokens** makes a token for scripts and integrations, sent as `Authorization: Bearer mise_…`.
Read tokens can only read. Tokens are shown once and stored as hashes; a token cannot create other tokens.

## Behind a reverse proxy

Set `PUBLIC_BASE_URL` to the outside address. With an `https` URL, session cookies are marked Secure. Without it,
mise works on the home network as is.
