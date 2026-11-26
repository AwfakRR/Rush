<script lang="ts">
  let count: number = $state(0);
</script>

<h1>Count with Svelte</h1>
<p>Edit this component while the counter is running.</p>
<button type="button" onclick={() => count++}>Count: {count}</button>

<style>
  button { background: #c2410c; }
  button:hover { background: #9a3412; }
</style>
