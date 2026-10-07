/**
 * A verbatim (trimmed) `GET /.mbr/site.json` from the Rust server with the
 * built-in relationship types, captured when the contacts halves were
 * integrated. Kept as server output, not rebuilt by `buildSiteNotes`, so the
 * charts are tested against the wire shape the server actually emits:
 * `inverse: null` on symmetric types, `attributes`, and a derived `employer`
 * edge implied by Zoe's `company: "[[Acme Corp]]"` with no `relationships:`
 * entry. Only notes Acme, Ada, Bob and Zoe are kept, and only edges among them.
 *
 * Regenerate by serving a repo with those four notes and copying the entries.
 */
export const SERVER_SITE_JSON = {
  "relationship_types": [
    {
      "name": "assistant",
      "symmetric": false,
      "inverse": "assists",
      "label": "Assistant",
      "label_plural": "Assistants",
      "hierarchy": "down",
      "category": "work"
    },
    {
      "name": "assists",
      "symmetric": false,
      "inverse": "assistant",
      "label": "Assists",
      "label_plural": "Assists",
      "hierarchy": "up",
      "category": "work"
    },
    {
      "name": "child",
      "symmetric": false,
      "inverse": "parent",
      "label": "Child",
      "label_plural": "Children",
      "hierarchy": "down",
      "category": "family"
    },
    {
      "name": "colleague",
      "symmetric": true,
      "inverse": null,
      "label": "Colleague",
      "label_plural": "Colleagues",
      "category": "work"
    },
    {
      "name": "employee",
      "symmetric": false,
      "inverse": "employer",
      "label": "Employee",
      "label_plural": "Employees",
      "hierarchy": "down",
      "category": "work"
    },
    {
      "name": "employer",
      "symmetric": false,
      "inverse": "employee",
      "label": "Employer",
      "label_plural": "Employers",
      "hierarchy": "up",
      "category": "work"
    },
    {
      "name": "manages",
      "symmetric": false,
      "inverse": "reports_to",
      "label": "Manages",
      "label_plural": "Manages",
      "hierarchy": "down",
      "category": "work"
    },
    {
      "name": "parent",
      "symmetric": false,
      "inverse": "child",
      "label": "Parent",
      "label_plural": "Parents",
      "hierarchy": "up",
      "category": "family"
    },
    {
      "name": "reports_to",
      "symmetric": false,
      "inverse": "manages",
      "label": "Reports to",
      "label_plural": "Reports to",
      "hierarchy": "up",
      "category": "work"
    },
    {
      "name": "sibling",
      "symmetric": true,
      "inverse": null,
      "label": "Sibling",
      "label_plural": "Siblings",
      "category": "family"
    },
    {
      "name": "spouse",
      "symmetric": true,
      "inverse": null,
      "label": "Spouse",
      "label_plural": "Spouses",
      "category": "family"
    }
  ],
  "markdown_files": [
    {
      "url_path": "/orgs/acme/",
      "frontmatter": {
        "title": "Acme Corp",
        "type": "organization"
      },
      "relationships": [
        {
          "rel_type": "employer",
          "predicate": "employee",
          "neighbor": "/people/ada/",
          "neighbor_title": "Ada King",
          "neighbor_raw": "Ada King",
          "resolved": true,
          "direction": "incoming",
          "attributes": {},
          "derived": true
        },
        {
          "rel_type": "employer",
          "predicate": "employee",
          "neighbor": "/people/bob/",
          "neighbor_title": "Bob Stone",
          "neighbor_raw": "Bob Stone",
          "resolved": true,
          "direction": "incoming",
          "attributes": {},
          "derived": true
        },
        {
          "rel_type": "employer",
          "predicate": "employee",
          "neighbor": "/people/zoe/",
          "neighbor_title": "Zoe Park",
          "neighbor_raw": "Zoe Park",
          "resolved": true,
          "direction": "incoming",
          "attributes": {},
          "derived": true
        }
      ]
    },
    {
      "url_path": "/people/ada/",
      "frontmatter": {
        "company": "[[Acme Corp]]",
        "dates.birthday": "1961-02-14",
        "job_title": "Chief Executive Officer",
        "title": "Ada King",
        "type": "person"
      },
      "relationships": [
        {
          "rel_type": "employer",
          "predicate": "employer",
          "neighbor": "/orgs/acme/",
          "neighbor_title": "Acme Corp",
          "neighbor_raw": "[[Acme Corp]]",
          "resolved": true,
          "direction": "outgoing",
          "attributes": {},
          "derived": false
        },
        {
          "rel_type": "reports_to",
          "predicate": "manages",
          "neighbor": "/people/bob/",
          "neighbor_title": "Bob Stone",
          "neighbor_raw": "Bob Stone",
          "resolved": true,
          "direction": "incoming",
          "attributes": {},
          "derived": true
        }
      ]
    },
    {
      "url_path": "/people/bob/",
      "frontmatter": {
        "company": "[[Acme Corp]]",
        "dates.birthday": "1975-06-02",
        "department": "Engineering",
        "job_title": "VP Engineering",
        "title": "Bob Stone",
        "type": "person"
      },
      "relationships": [
        {
          "rel_type": "employer",
          "predicate": "employer",
          "neighbor": "/orgs/acme/",
          "neighbor_title": "Acme Corp",
          "neighbor_raw": "[[Acme Corp]]",
          "resolved": true,
          "direction": "outgoing",
          "attributes": {},
          "derived": false
        },
        {
          "rel_type": "reports_to",
          "predicate": "manages",
          "neighbor": "/people/zoe/",
          "neighbor_title": "Zoe Park",
          "neighbor_raw": "Zoe Park",
          "resolved": true,
          "direction": "incoming",
          "attributes": {},
          "derived": true
        },
        {
          "rel_type": "reports_to",
          "predicate": "reports_to",
          "neighbor": "/people/ada/",
          "neighbor_title": "Ada King",
          "neighbor_raw": "[[Ada King]]",
          "resolved": true,
          "direction": "outgoing",
          "attributes": {},
          "derived": false
        }
      ]
    },
    {
      "url_path": "/people/zoe/",
      "frontmatter": {
        "company": "[[Acme Corp]]",
        "department": "Engineering",
        "job_title": "Staff Engineer",
        "title": "Zoe Park",
        "type": "person"
      },
      "relationships": [
        {
          "rel_type": "employer",
          "predicate": "employer",
          "neighbor": "/orgs/acme/",
          "neighbor_title": "Acme Corp",
          "neighbor_raw": "[[Acme Corp]]",
          "resolved": true,
          "direction": "outgoing",
          "attributes": {},
          "derived": true
        },
        {
          "rel_type": "reports_to",
          "predicate": "reports_to",
          "neighbor": "/people/bob/",
          "neighbor_title": "Bob Stone",
          "neighbor_raw": "[[Bob Stone]]",
          "resolved": true,
          "direction": "outgoing",
          "attributes": {},
          "derived": false
        }
      ]
    }
  ]
} as const
