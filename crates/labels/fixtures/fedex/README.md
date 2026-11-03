# FedEx request/answer fixtures

Shapes come from FedEx's published API files (fetched 2026-11-03, no API
calls made):

- `ship_request_documented.json`: Ship API Postman collection request
  `SH0018_US_US_USE_SCHEDULED_PICKUP_FEDEX_GROUND_YOUR_PACKAGING_SENDER_URL_ONLY_PAPER_85X11_TOP_HALF_LABEL`
  (https://developer.fedex.com/wirc/jsonapi-collections/api_groups/ShipAPI/Ship%20API_collection.zip),
  verbatim apart from formatting.
- `rate_request_documented.json`: Rate API `MinimumSamplePayload_1`
  (https://developer.fedex.com/wirc/json/api_groups/Rate/RateQuotes-Resource.json).
- `cancel_request_documented.json`: Ship API `Cancel_Shipment` sample
  (https://developer.fedex.com/wirc/json/api_groups/Ship/Shipment-Resource.json).
- `ship_reply.json`, `rate_reply.json`, `cancel_reply.json`: answers built
  on the response schemas in those two OpenAPI files. FedEx publishes no
  example answer with values, so every value here is synthetic; the label is
  a base64 `%PDF-` stub.

Doc pages: https://developer.fedex.com/api/en-us/catalog/ship/v1/docs.html
and https://developer.fedex.com/api/en-us/catalog/rate/v1/docs.html.
