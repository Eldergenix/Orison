# API and Data Framework

## OpenAPI import

```bash
ori schema import openapi ./stripe.yaml --module vendor.stripe
```

Generated functions include capability annotations.

```ori
fn create_payment_intent(input: PaymentIntentCreate)
  -> Result[PaymentIntent, StripeErr]
  uses StripeApi
```

## GraphQL import

```bash
ori schema import graphql ./schema.graphql --module vendor.github
```

## SQL query declarations

```ori
query FindUser(id: UserId) -> Option[User]:
  select id, name, email
  from users
  where id = $id
```

## Migrations

```ori
migration AddUsers:
  create table users:
    id UUID primary
    name Str not null
    email Str unique not null
```

## Agent value

Agents can inspect typed schemas and query declarations instead of inferring database/API structure from strings.
