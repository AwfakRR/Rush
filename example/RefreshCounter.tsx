import React from 'react';

export default function RefreshCounter() {
    const [count, setCount] = React.useState(0);
    return (
        <button onClick={() => setCount(count + 1)}>
            React clicks: {count}
        </button>
    );
}
