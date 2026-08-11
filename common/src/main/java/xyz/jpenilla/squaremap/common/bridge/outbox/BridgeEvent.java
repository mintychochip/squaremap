package xyz.jpenilla.squaremap.common.bridge.outbox;

import java.util.Objects;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;

/** Immutable bridge updates that can be coalesced without losing durable state. */
public sealed interface BridgeEvent permits BridgeEvent.ReplaceState, BridgeEvent.DirtyChunk, BridgeEvent.ResyncWorld, BridgeEvent.Control {
    record WorldKey(String namespace, String value) implements Comparable<WorldKey> {
        public WorldKey {
            Objects.requireNonNull(namespace, "namespace");
            Objects.requireNonNull(value, "value");
        }

        @Override
        public int compareTo(final WorldKey other) {
            final int namespaceResult = this.namespace.compareTo(other.namespace);
            return namespaceResult != 0 ? namespaceResult : this.value.compareTo(other.value);
        }
    }

    record ReplaceState(String key, Envelope payload) implements BridgeEvent {
        public ReplaceState {
            Objects.requireNonNull(key, "key");
            Objects.requireNonNull(payload, "payload");
        }
    }

    record DirtyChunk(WorldKey world, long epoch, int x, int z, long revision) implements BridgeEvent {
        public DirtyChunk {
            Objects.requireNonNull(world, "world");
            if (epoch < 0 || revision < 0) throw new IllegalArgumentException("epoch and revision must be non-negative");
        }
    }

    record ResyncWorld(WorldKey world, long epoch) implements BridgeEvent {
        public ResyncWorld {
            Objects.requireNonNull(world, "world");
            if (epoch < 0) throw new IllegalArgumentException("epoch must be non-negative");
        }
    }

    record Control(long correlationId, Envelope payload) implements BridgeEvent {
        public Control {
            Objects.requireNonNull(payload, "payload");
            if (correlationId <= 0) throw new IllegalArgumentException("correlation ID must be positive");
            if (!payload.hasControlRequest()) throw new IllegalArgumentException("control event requires ControlRequest payload");
        }
    }
}
