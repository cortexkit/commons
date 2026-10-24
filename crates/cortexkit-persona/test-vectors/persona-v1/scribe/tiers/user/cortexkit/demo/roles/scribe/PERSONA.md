---
version: 2
fragments: [rules, tone, mc/memory-profile@1]
disabled_blocks: [tools]
---
{% block closing %}{% include "tone" %}

{% include "mc/memory-profile@1" %}

{{ super() }}{% endblock %}
