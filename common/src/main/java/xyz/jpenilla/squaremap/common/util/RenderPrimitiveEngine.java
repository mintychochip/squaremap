package xyz.jpenilla.squaremap.common.util;

public final class RenderPrimitiveEngine {
    private RenderPrimitiveEngine() {}

    public static final class Rectangle {
        private final int minX, minZ, maxX, maxZ;
        private final int minChunkX, minChunkZ, maxChunkX, maxChunkZ;
        private final int minRegionX, minRegionZ, maxRegionX, maxRegionZ;

        private Rectangle(final int minX, final int minZ, final int maxX, final int maxZ) {
            this.minX = minX; this.minZ = minZ; this.maxX = maxX; this.maxZ = maxZ;
            this.minChunkX = Numbers.blockToChunk(minX); this.minChunkZ = Numbers.blockToChunk(minZ);
            this.maxChunkX = Numbers.blockToChunk(maxX); this.maxChunkZ = Numbers.blockToChunk(maxZ);
            this.minRegionX = Numbers.blockToRegion(minX); this.minRegionZ = Numbers.blockToRegion(minZ);
            this.maxRegionX = Numbers.blockToRegion(maxX); this.maxRegionZ = Numbers.blockToRegion(maxZ);
        }
        public int minX() { return minX; } public int minZ() { return minZ; }
        public int maxX() { return maxX; } public int maxZ() { return maxZ; }
        public int minChunkX() { return minChunkX; } public int minChunkZ() { return minChunkZ; }
        public int maxChunkX() { return maxChunkX; } public int maxChunkZ() { return maxChunkZ; }
        public int minRegionX() { return minRegionX; } public int minRegionZ() { return minRegionZ; }
        public int maxRegionX() { return maxRegionX; } public int maxRegionZ() { return maxRegionZ; }
    }

    public static final class Circle {
        private final int cx, cz;
        private final long radiusSquared;
        private Circle(final int cx, final int cz, final int radius) {
            this.cx = cx; this.cz = cz; this.radiusSquared = (long) radius * radius;
        }
        public int cx() { return cx; } public int cz() { return cz; }
        public long radiusSquared() { return radiusSquared; }
    }

    public static final class Polygon {
        private final int[] xs, zs;
        private final int minX, minZ, maxX, maxZ;
        private Polygon(final int[] xs, final int[] zs, final int minX, final int minZ, final int maxX, final int maxZ) {
            this.xs = xs.clone(); this.zs = zs.clone();
            this.minX = minX; this.minZ = minZ; this.maxX = maxX; this.maxZ = maxZ;
        }
        public int[] xs() { return xs.clone(); } public int[] zs() { return zs.clone(); }
        public int minX() { return minX; } public int minZ() { return minZ; }
        public int maxX() { return maxX; } public int maxZ() { return maxZ; }
    }

    /** The block interval is [minX, maxX) x [minZ, maxZ). */
    public static final class WorldBorder {
        private final int centerX, centerZ, radius, minX, minZ, maxX, maxZ;
        private WorldBorder(final int centerX, final int centerZ, final int radius,
                            final int minX, final int minZ, final int maxX, final int maxZ) {
            this.centerX = centerX; this.centerZ = centerZ; this.radius = radius;
            this.minX = minX; this.minZ = minZ; this.maxX = maxX; this.maxZ = maxZ;
        }
        public int centerX() { return centerX; } public int centerZ() { return centerZ; }
        public int radius() { return radius; } public int minX() { return minX; } public int minZ() { return minZ; }
        public int maxX() { return maxX; } public int maxZ() { return maxZ; }
    }

    public static Rectangle rectangle(final int minX, final int minZ, final int maxX, final int maxZ) {
        if (minX >= maxX || minZ >= maxZ) {
            throw new IllegalArgumentException("rectangle bounds must be positive");
        }
        return new Rectangle(minX, minZ, maxX, maxZ);
    }

    public static boolean rectangleBlock(final Rectangle r, final int x, final int z) {
        return x >= r.minX() && x <= r.maxX() && z >= r.minZ() && z <= r.maxZ();
    }

    public static boolean rectangleChunk(final Rectangle r, final int cx, final int cz) {
        return cx >= r.minChunkX() && cx <= r.maxChunkX()
            && cz >= r.minChunkZ() && cz <= r.maxChunkZ();
    }

    public static boolean rectangleRegion(final Rectangle r, final int rx, final int rz) {
        return rx >= r.minRegionX() && rx <= r.maxRegionX()
            && rz >= r.minRegionZ() && rz <= r.maxRegionZ();
    }

    public static int rectangleCount(final Rectangle r, final int rx, final int rz) {
        final int minChunkX = rx * 32;
        final int minChunkZ = rz * 32;
        final int maxChunkX = minChunkX + 31;
        final int maxChunkZ = minChunkZ + 31;
        return overlapCount(minChunkX, maxChunkX, r.minChunkX(), r.maxChunkX())
            * overlapCount(minChunkZ, maxChunkZ, r.minChunkZ(), r.maxChunkZ());
    }

    public static Circle circle(final int cx, final int cz, final int radius) {
        if (radius < 1) {
            throw new IllegalArgumentException("radius must be positive");
        }
        return new Circle(cx, cz, radius);
    }

    public static boolean circleBlock(final Circle c, final int x, final int z) {
        final long dx = (long) x - c.cx();
        final long dz = (long) z - c.cz();
        return dx * dx + dz * dz <= c.radiusSquared();
    }

    public static boolean circleChunk(final Circle c, final int cx, final int cz) {
        long bx = cx * 16L;
        long bz = cz * 16L;
        if (bx < c.cx()) bx += Math.min(15L, (long) c.cx() - bx);
        if (bz < c.cz()) bz += Math.min(15L, (long) c.cz() - bz);
        return distanceSquared(bx - c.cx(), bz - c.cz()) <= c.radiusSquared();
    }

    public static boolean circleRegion(final Circle c, final int rx, final int rz) {
        long bx = rx * 512L;
        long bz = rz * 512L;
        if (bx < c.cx()) bx += Math.min(511L, (long) c.cx() - bx);
        if (bz < c.cz()) bz += Math.min(511L, (long) c.cz() - bz);
        return distanceSquared(bx - c.cx(), bz - c.cz()) <= c.radiusSquared();
    }

    public static int circleCount(final Circle c, final int rx, final int rz) {
        final int startX = rx * 32;
        final int startZ = rz * 32;
        if (circleChunk(c, startX, startZ)
            && circleChunk(c, startX + 31, startZ)
            && circleChunk(c, startX, startZ + 31)
            && circleChunk(c, startX + 31, startZ + 31)) {
            return 1024;
        }
        int count = 0;
        for (int x = 0; x < 32; x++) {
            for (int z = 0; z < 32; z++) {
                if (circleChunk(c, startX + x, startZ + z)) count++;
            }
        }
        return count;
    }

    public static Polygon polygon(final int[][] points) {
        final int[] xs = new int[points.length];
        final int[] zs = new int[points.length];
        int minX = Integer.MAX_VALUE;
        int minZ = Integer.MAX_VALUE;
        int maxX = Integer.MIN_VALUE;
        int maxZ = Integer.MIN_VALUE;
        for (int i = 0; i < points.length; i++) {
            xs[i] = points[i][0];
            zs[i] = points[i][1];
            minX = Math.min(minX, xs[i]);
            minZ = Math.min(minZ, zs[i]);
            maxX = Math.max(maxX, xs[i]);
            maxZ = Math.max(maxZ, zs[i]);
        }
        return new Polygon(xs, zs, minX, minZ, maxX, maxZ);
    }

    public static boolean polygonBlock(final Polygon p, final int x, final int z) {
        final int n = p.xs.length;
        if (n <= 2 || x < p.minX || z < p.minZ || x >= p.minX + (p.maxX - p.minX)
            || z >= p.minZ + (p.maxZ - p.minZ)) {
            return false;
        }
        int hits = 0;
        int lastX = p.xs[n - 1];
        int lastZ = p.zs[n - 1];
        for (int i = 0; i < n; i++) {
            final int curX = p.xs[i];
            final int curZ = p.zs[i];
            if (curZ == lastZ) {
                lastX = curX;
                lastZ = curZ;
                continue;
            }
            final int leftX;
            if (curX < lastX) {
                if (x >= lastX) {
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                leftX = curX;
            } else {
                if (x >= curX) {
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                leftX = lastX;
            }
            final double test1;
            final double test2;
            if (curZ < lastZ) {
                if (z < curZ || z >= lastZ) {
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                if (x < leftX) {
                    hits++;
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                test1 = x - curX;
                test2 = z - curZ;
            } else {
                if (z < lastZ || z >= curZ) {
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                if (x < leftX) {
                    hits++;
                    lastX = curX;
                    lastZ = curZ;
                    continue;
                }
                test1 = x - lastX;
                test2 = z - lastZ;
            }
            if (test1 < test2 / (lastZ - curZ) * (lastX - curX)) hits++;
            lastX = curX;
            lastZ = curZ;
        }
        return (hits & 1) != 0;
    }

    public static boolean polygonChunk(final Polygon p, final int cx, final int cz) {
        final int minX = Numbers.chunkToBlock(cx);
        final int minZ = Numbers.chunkToBlock(cz);
        for (int x = minX; x < minX + 16; x++) {
            for (int z = minZ; z < minZ + 16; z++) {
                if (polygonBlock(p, x, z)) return true;
            }
        }
        return false;
    }

    public static boolean polygonRegion(final Polygon p, final int rx, final int rz) {
        final int minX = Numbers.regionToChunk(rx);
        final int minZ = Numbers.regionToChunk(rz);
        for (int x = minX; x < minX + 32; x++) {
            for (int z = minZ; z < minZ + 32; z++) {
                if (polygonChunk(p, x, z)) return true;
            }
        }
        return false;
    }

    public static int polygonCount(final Polygon p, final int rx, final int rz) {
        final int minX = Numbers.regionToChunk(rx);
        final int minZ = Numbers.regionToChunk(rz);
        int count = 0;
        for (int x = minX; x < minX + 32; x++) {
            for (int z = minZ; z < minZ + 32; z++) {
                if (polygonChunk(p, x, z)) count++;
            }
        }
        return count;
    }

    public static WorldBorder worldBorder(final int centerX, final int centerZ, final int radius) {
        if (radius < 0) throw new IllegalArgumentException("radius must not be negative");
        final long minX = (long) centerX - radius;
        final long minZ = (long) centerZ - radius;
        final long maxX = (long) centerX + radius;
        final long maxZ = (long) centerZ + radius;
        if (minX < Integer.MIN_VALUE || minZ < Integer.MIN_VALUE || maxX > Integer.MAX_VALUE || maxZ > Integer.MAX_VALUE) {
            throw new IllegalArgumentException("world border bounds overflow");
        }
        return new WorldBorder(centerX, centerZ, radius, (int) minX, (int) minZ, (int) maxX, (int) maxZ);
    }

    public static WorldBorder fromRuntime(final double centerX, final double centerZ, final double size) {
        if (!Double.isFinite(centerX) || !Double.isFinite(centerZ) || !Double.isFinite(size) || size < 0) {
            throw new IllegalArgumentException("world border runtime values must be finite and non-negative");
        }
        final double truncatedX = Math.copySign(Math.floor(Math.abs(centerX)), centerX);
        final double truncatedZ = Math.copySign(Math.floor(Math.abs(centerZ)), centerZ);
        final double radius = Math.ceil(size / 2);
        if (truncatedX < Integer.MIN_VALUE || truncatedX > Integer.MAX_VALUE
            || truncatedZ < Integer.MIN_VALUE || truncatedZ > Integer.MAX_VALUE
            || radius > Integer.MAX_VALUE) {
            throw new IllegalArgumentException("world border runtime values overflow");
        }
        return worldBorder((int) truncatedX, (int) truncatedZ, (int) radius);
    }

    public static boolean worldBorderBlock(final WorldBorder b, final int x, final int z) {
        return x >= b.minX() && x < b.maxX() && z >= b.minZ() && z < b.maxZ();
    }

    public static boolean worldBorderChunk(final WorldBorder b, final int cx, final int cz) {
        final int minX = Numbers.blockToChunk(b.minX());
        final int maxX = Numbers.blockToChunk(b.maxX());
        final int minZ = Numbers.blockToChunk(b.minZ());
        final int maxZ = Numbers.blockToChunk(b.maxZ());
        return cx >= minX && cx <= maxX && cz >= minZ && cz <= maxZ;
    }

    public static boolean worldBorderRegion(final WorldBorder b, final int rx, final int rz) {
        final int minX = Numbers.blockToRegion(b.minX());
        final int maxX = Numbers.blockToRegion(b.maxX());
        final int minZ = Numbers.blockToRegion(b.minZ());
        final int maxZ = Numbers.blockToRegion(b.maxZ());
        return rx >= minX && rx <= maxX && rz >= minZ && rz <= maxZ;
    }

    public static int worldBorderCount(final WorldBorder b, final int rx, final int rz) {
        final int minX = rx * 32;
        final int minZ = rz * 32;
        final int maxX = minX + 31;
        final int maxZ = minZ + 31;
        final int borderMinX = Numbers.blockToChunk(b.minX());
        final int borderMaxX = Numbers.blockToChunk(b.maxX());
        final int borderMinZ = Numbers.blockToChunk(b.minZ());
        final int borderMaxZ = Numbers.blockToChunk(b.maxZ());
        return overlapCount(minX, maxX, borderMinX, borderMaxX)
            * overlapCount(minZ, maxZ, borderMinZ, borderMaxZ);
    }

    public static int parity(final int x, final int z) {
        return (x + z) & 1;
    }

    public static int terrain(final int currentY, final int previousY, final int color, final int odd) {
        final double diffY = ((double) currentY - previousY) + ((double) odd - 0.5D) * 0.4D;
        final byte shade = (byte) (diffY > 0.6D ? 2 : (diffY < -0.6D ? 0 : 1));
        return Colors.shade(color, shade);
    }

    public static int depth(final double depth, final int color, final double odd) {
        final double diffY = depth * 0.1D + odd * 0.2D;
        final byte shade = (byte) (diffY < 0.5D ? 2 : (diffY > 0.9D ? 0 : 1));
        return Colors.shade(color, shade);
    }

    public static int fluid(final int depth, int color, final boolean water, final int under,
                            final boolean waterChecker, final boolean waterClear,
                            final boolean lavaChecker, final int odd) {
        boolean shaded = false;
        if (water) {
            if (waterChecker) {
                color = depth(depth, color, odd);
                shaded = true;
            }
            if (waterClear) {
                if (!waterChecker) color = Colors.shade(color, 0.85F - (depth * 0.01F));
                color = Colors.mix(color, under, 0.20F / (depth / 2.0F));
                shaded = true;
            }
        } else if (lavaChecker) {
            color = depth(depth, color, odd);
            shaded = true;
        }
        return shaded ? color : Colors.removeAlpha(color);
    }

    public static int glass(final int under, final int glass, final float alpha) {
        return Colors.mix(under, glass, alpha);
    }

    public enum FluidKind { WATER, LAVA }

    public static FluidKind classifyUnknownFluid(final int renderedColor, final boolean nativeWater, final boolean nativeLava) {
        if (nativeWater) return FluidKind.WATER;
        if (nativeLava) return FluidKind.LAVA;
        return (renderedColor >> 24 & 255) == 255 ? FluidKind.LAVA : FluidKind.WATER;
    }

    private static long distanceSquared(final long x, final long z) {
        return x * x + z * z;
    }

    private static int overlapCount(final int minA, final int maxA, final int minB, final int maxB) {
        final int width = Math.min(maxA, maxB) - Math.max(minA, minB) + 1;
        return Math.max(0, width);
    }
}
