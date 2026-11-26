<script setup lang="ts">
import { ref } from 'vue';

const count = ref<number>(0);
function increment(): void {
  count.value++;
}
</script>

<template>
  <h1>Count with Vue</h1>
  <p>Edit this component while the counter is running.</p>
  <button type="button" @click="increment">Count: {{ count }}</button>
</template>

<style scoped>
button { background: #047857; }
button:hover { background: #065f46; }
</style>
