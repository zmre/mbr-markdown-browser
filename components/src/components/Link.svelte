<svelte:options customElement={{ tag: "mbr-a" }} />

<script lang="ts">
  let {
    href = "",
    rel,
    target,
  }: { href: string; rel?: string; target?: string } = $props();
</script>

<a {href} {rel} {target}><slot /></a>
