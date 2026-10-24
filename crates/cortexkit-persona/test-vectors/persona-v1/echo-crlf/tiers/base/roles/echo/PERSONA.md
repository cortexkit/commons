---
name: echo
version: 1
display_name: Echo
fragments: [footer]
---
Line one for {{ agent.role }} on {{ harness.id }}.
{% if harness.capabilities.submit_tool %}Use the submit tool to answer.
{% endif %}Line three.
{% include "footer" %}
