# Signing in

Every page and API route except the health check (`/api/v1/health`) needs a signed-in account. A fresh install opens on a welcome page
where the first visitor creates the admin account and the first household; after that, admins add people from
**Account → People**, or from the server with `mise user`.

## Accounts and households

A household shares plans, favourites, lists and its own recipes, which no other household sees; every account
belongs to one. Admins can move people between households and add new ones.

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

- **Who gets in.** By default, only people who already have an account here: an admin adds one (**Account → People**,
  or `mise user add`) with the person's email, and their first sign-on links to it when the provider says that email
  is verified. To let people make an account by signing on, set `OIDC_ALLOWED_GROUP`: its members (and
  `OIDC_ADMIN_GROUP`'s) get one at their first sign-in, and nobody else signs in at all, so leaving the group at the
  provider ends access. `OIDC_OPEN_SIGNUP=true` gives an account to anyone the provider signs in; use it only with a
  provider whose every user is yours (a household's own Authentik), never a public one such as Google, where it
  would let anyone with a Google account in.
- A returning person is recognised by their provider identity. A new account goes in a household of its own, or in
  `OIDC_HOUSEHOLD` when set.
- A sign-on never links to an admin's account by email: whoever controls that address at the provider would get the
  admin's rights. An admin signs in with their password, or gets admin rights through `OIDC_ADMIN_GROUP`.
- With `OIDC_ADMIN_GROUP`, admin rights follow membership of that group (the `groups` claim; in Authentik and
  Keycloak add a groups mapper or scope) at every sign-in.
- The flow uses PKCE and a nonce, and finishes only in the browser that started it (a cookie holds the sign-in's
  state); sign-in attempts expire after 15 minutes and can be used once. After signing in, mise returns only to an
  address on this site.

## API tokens

**Account → API tokens** makes a token for scripts and integrations, sent as `Authorization: Bearer mise_…`.
Read tokens can only read. Tokens are shown once and stored as hashes; a token cannot create other tokens.

## Kitchen devices

A screen in the kitchen, such as an Echo Show, whose browser can't sign in comfortably. An admin adds one under
**Account → Kitchen devices** with a name and gets a link, shown once: open it on the device within the hour. It
works once, and sets a cookie on the device that lasts 400 days from its last use. The device reads the household's
plan, dishes, versions and photos and nothing else: no shopping list, accounts, names or emails, and no changes.
It opens kitchen mode, the week's dishes a step at a time, and nothing else of the app.
**Revoke** signs it out at its next request. A browser signed in to an account stays signed in when a kitchen link
is opened in it, and acts as that account until it signs out. Open the link on the device itself: an app that
previews links may open it first, and then it no longer works.

## Behind a reverse proxy

Set `PUBLIC_BASE_URL` to the outside address. With an `https` URL, session cookies are marked Secure. Without it,
mise works on the home network as is.
