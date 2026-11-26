import styles from './card.module.css';

document.getElementById('about').classList.add(styles.card);
const filename = new URL(import.meta.url).pathname.split('/').pop();
document.getElementById('about-status').textContent = `Page entry loaded: ${filename}`;
