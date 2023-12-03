# Authorization

After visiting `/authorize` user will be redirected to the google oauth login page.
After logging in, user will be redirected to `/oauth` with some query parameters.
The query parameters are used to get the access token from google.
Code will check if the user has access to the application by checking if the user's email is in the `ALLOWED_USERS` list.
If the user is allowed code will check if the user is already in the database.
If the user is in the database, simply redirect to `/dashboard?code={}` with the user's code.
Otherwise create a new user, start refresh loop and redirect to `/dashboard?code={}` with the user's code.

## Note - user token

User token is constant value that is used to identify the user. It is not used for sse authentication.

## Note - access code

The access code (also called just code) is a special value that is generated after the user logs in.
It binds to the user's token and is deleted after the initial authentication or after one minute.

After redirecting to `/dashboard` the code is used to get the user's token.
Client calls `/api/auth` with the code as a body. In response the server returns the user's token.
