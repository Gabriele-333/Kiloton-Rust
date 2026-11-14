import net.gabriele333.kiloton.blast.LateGrid;
import net.gabriele333.kiloton.fx.MushroomCloud;
import net.gabriele333.kiloton.fx.SkyLight;
import net.gabriele333.kiloton.fx.ViewerEffects;
import net.gabriele333.kiloton.fx.WilsonCloud;
import net.gabriele333.kiloton.blast.RingColumns;
import net.gabriele333.kiloton.physics.Ballistics;
import net.gabriele333.kiloton.physics.BlackRain;
import net.gabriele333.kiloton.physics.BlastModel;
import net.gabriele333.kiloton.physics.BurstDetection;
import net.gabriele333.kiloton.physics.Fallout;
import net.gabriele333.kiloton.physics.FissionModel;
import net.gabriele333.kiloton.physics.Intercept;
import net.gabriele333.kiloton.physics.RadarHorizon;
import net.gabriele333.kiloton.physics.SamGeometry;
import net.gabriele333.kiloton.physics.ThermonuclearModel;
import net.gabriele333.kiloton.physics.Warhead;

import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Reference tables for the Rust port of Kiloton's physics: every public function of
 * {@code net.gabriele333.kiloton.physics}, evaluated by the Java code itself over many inputs.
 * One line per evaluation: {@code name<TAB>args...<TAB>|<TAB>results...}; doubles as
 * {@link Double#toString} (they read back exactly), booleans as true/false, a missing value as null.
 *
 * <p>Usage: {@code java -cp <classes> ParityTables <output file>}.
 */
public final class ParityTables {
    private final PrintWriter out;
    private int lines;

    private ParityTables(PrintWriter out) {
        this.out = out;
    }

    public static void main(String[] args) throws IOException {
        Path file = Path.of(args.length > 0 ? args[0] : "physics.tsv");
        Files.createDirectories(file.toAbsolutePath().getParent());
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(file, StandardCharsets.UTF_8))) {
            ParityTables t = new ParityTables(w);
            t.blast();
            t.fallout();
            t.blackRain();
            t.fission();
            t.thermonuclear();
            t.ballistics();
            t.sam();
            t.intercept();
            t.radar();
            t.burstDetection();
            t.rings();
            t.lateGrid();
            System.out.println("wrote " + t.lines + " lines to " + file);
        }
    }

    // ---- output -------------------------------------------------------------------------------

    private static String s(Object v) {
        if (v == null) return "null";
        if (v instanceof Double d) return Double.toString(d);
        if (v instanceof Float f) return Double.toString(f.doubleValue());
        if (v instanceof double[] a) {
            StringBuilder b = new StringBuilder();
            for (int i = 0; i < a.length; i++) {
                if (i > 0) b.append('\t');
                b.append(Double.toString(a[i]));
            }
            return b.toString();
        }
        return v.toString();
    }

    private void row(String name, Object[] in, Object... res) {
        StringBuilder b = new StringBuilder(name);
        for (Object o : in) b.append('\t').append(s(o));
        b.append("\t|");
        for (Object o : res) b.append('\t').append(s(o));
        out.println(b);
        lines++;
    }

    private static Object[] in(Object... a) {
        return a;
    }

    // ---- inputs -------------------------------------------------------------------------------

    static final double[] YIELDS = {0.0, 0.001, 0.0123, 0.05, 0.3, 0.5, 1.0, 2.5, 7.0, 15.0, 20.0, 52.4, 115.0, 150.0,
            500.0, 1000.0, 3333.3, 15000.0, 20000.0, 50000.0};
    static final double[] FISSION = {1.0, 0.5, 0.02, 0.0, 1.5, 0.75};
    static final double[] DISTANCES = {-5.0, 0.0, 0.01, 0.05, 0.5, 1.0, 3.7, 10.0, 18.0, 30.0, 55.5, 100.0, 250.0, 335.0,
            460.0, 777.7, 1000.0, 1350.0, 2500.0, 4000.0, 10000.0, 25000.0, 60000.0, 100000.0, 250000.0};
    static final double[] TIMES = {-1.0, 0.0, 0.001, 0.01, 0.1, 0.25, 0.5, 1.0, 2.0, 3.3, 5.0, 10.0, 30.0, 60.0, 146.0, 300.0};
    static final long[] SEEDS = {0L, 1L, -1L, 42L, 123456789L, Long.MIN_VALUE, Long.MAX_VALUE, 0x123456789ABCDEFL,
            -7046029254386353131L, 987654321987654321L};

    // ---- BlastModel ---------------------------------------------------------------------------

    private void blast() {
        for (double y : YIELDS) {
            for (double f : FISSION) {
                BlastModel m = new BlastModel(y, f);
                Object[] key = in(y, f);
                row("blast.model", key, m.yieldKt, m.fissionFraction, m.cubeRoot, m.craterRadius, m.craterDepth,
                        m.lipHeight, m.fireballRadius, m.fireballTime, m.naturalEffectRadius, m.thermonuclear());
                if (f != 1.0 && f != 0.5) continue; // the rest depends on the yield only
                List<Double> ds = new ArrayList<>();
                for (double d : DISTANCES) ds.add(d);
                for (double k : new double[]{0.1, 0.33, 0.5, 0.9, 0.999, 1.0, 1.001, 1.1, 1.25, 1.3, 2.0}) {
                    ds.add(m.naturalEffectRadius * k);
                }
                for (double k : new double[]{0.0, 0.25, 0.5, 0.99, 1.0, 1.5, 3.0}) ds.add(m.craterRadius * k);
                for (double d : ds) {
                    row("blast.at", in(y, f, d), m.overpressurePsi(d), m.dynamicPressurePsi(d), m.windSpeed(d),
                            m.shockSpeed(d), m.arrivalTime(d), m.thermalFluence(d), m.initialDoseRem(d),
                            m.craterDepthAt(d), m.lipHeightAt(d));
                }
                List<Double> ts = new ArrayList<>();
                for (double t : TIMES) ts.add(t);
                for (double k : new double[]{0.0, 1.0e-6, 0.5, 0.999, 1.0, 2.0}) ts.add(m.fireballTime * k);
                double last = m.arrivalTime(m.naturalEffectRadius * 1.25 + 16.0);
                for (double k : new double[]{0.5, 0.99, 1.0, 1.01, 3.0}) ts.add(last * k);
                for (double t : ts) {
                    row("blast.time", in(y, f, t), m.frontRadius(t), m.fireballRadiusAt(t));
                }
                for (double p : new double[]{0.55, 1.0, 3.0, 5.0, 10.0, 20.0, 50.0, 200.0, 5000.0}) {
                    row("blast.psi", in(y, f, p), m.radiusForPsi(p));
                }
                for (double q : new double[]{0.5, 1.0, 6.0, 10.0, 20.0, 100.0, 1.0e5}) {
                    row("blast.fluence", in(y, f, q), m.radiusForFluence(q));
                }
                for (double rem : new double[]{1.0, 100.0, 500.0, 1000.0, 1.0e4, 1.0e7}) {
                    row("blast.dose", in(y, f, rem), m.radiusForDose(rem));
                }
            }
        }
        for (double p : new double[]{0.0, 0.5, 0.79, 0.8, 1.0, 2.5, 5.0, 20.0, 100.0, 1000.0}) {
            row("blast.damage", in(p), BlastModel.blastDamage(p));
        }
    }

    // ---- Fallout ------------------------------------------------------------------------------

    static final double[][] OFFSETS = {{0, 0}, {10, 0}, {0, -10}, {150, 80}, {-300, 450}, {1200, -50}, {-2500, -2500},
            {5000, 300}, {12000, -9000}, {40000, 1000}, {-60000, 25000}};

    private void fallout() {
        for (long seed : SEEDS) {
            row("fallout.wind", in(seed), Fallout.windAngle(seed), Fallout.windSpeed(seed));
        }
        for (double t : new double[]{0, 1, 100, 250, 999, 1000, 2000, 7000, 24000, 1.0e6}) {
            row("fallout.decay", in(t), Fallout.decay(t));
        }
        double[] ys = {0.01, 0.4, 1.0, 15.0, 115.0, 1000.0, 15000.0};
        double[] fs = {1.0, 0.5, 0.02};
        for (double y : ys) {
            for (double f : fs) {
                BlastModel m = new BlastModel(y, f);
                row("fallout.model", in(y, f), Fallout.plumeLength(m), Fallout.reach(m), Fallout.surgeRadius(m),
                        Fallout.dustReach(m));
                for (int si = 0; si < SEEDS.length; si += 3) {
                    long seed = SEEDS[si];
                    for (double[] o : OFFSETS) {
                        double dx = o[0] * m.cubeRoot, dz = o[1] * m.cubeRoot;
                        row("fallout.spot", in(y, f, seed, dx, dz), Fallout.doseRateH1(m, seed, dx, dz),
                                Fallout.arrivalSeconds(seed, dx, dz), Fallout.surgeH1(m, dx, dz),
                                Fallout.surgeSeconds(m, dx, dz), Fallout.dustCover(m, seed, dx, dz),
                                Fallout.dustSeconds(m, seed, dx, dz), Fallout.totalDose(m, seed, null, dx, dz));
                        for (double ticks : new double[]{0, 100, 600, 900, 1200, 2000, 5000, 20000, 200000}) {
                            row("fallout.rate", in(y, f, seed, dx, dz, ticks), Fallout.doseRate(m, seed, dx, dz, ticks));
                        }
                        for (double h : new double[]{0.1, 0.25, 0.5, 1.0, 3.0, 24.0, 1000.0}) {
                            row("fallout.until", in(y, f, seed, dx, dz, h), Fallout.doseUntil(m, seed, null, dx, dz, h));
                        }
                        for (double rem : new double[]{1.0, 50.0, 400.0, 1.0e4}) {
                            row("fallout.hours", in(y, f, seed, dx, dz, rem), Fallout.hoursToDose(m, seed, null, dx, dz, rem));
                        }
                    }
                }
            }
        }
    }

    // ---- BlackRain ----------------------------------------------------------------------------

    private void blackRain() {
        for (double y : new double[]{0.1, 0.5, 1.0, 15.0, 115.0, 2500.0}) {
            for (double f : new double[]{1.0, 0.5}) {
                BlastModel m = new BlastModel(y, f);
                for (long seed : SEEDS) {
                    for (double hum : new double[]{0.0, 0.2, 0.3, 0.4, 0.6, 0.8, 1.0}) {
                        BlackRain r = BlackRain.of(m, seed, hum);
                        if (r == null) {
                            row("rain.of", in(y, f, seed, hum), "null");
                            continue;
                        }
                        row("rain.of", in(y, f, seed, hum), "some", r.start, r.end, r.centre, r.along, r.across,
                                r.peakH1, r.reach());
                        for (double[] o : OFFSETS) {
                            double dx = o[0] * m.cubeRoot * 0.5, dz = o[1] * m.cubeRoot * 0.5;
                            row("rain.spot", in(y, f, seed, hum, dx, dz), r.footprint(dx, dz), r.rainoutH1(dx, dz),
                                    Fallout.totalDose(m, seed, r, dx, dz),
                                    Fallout.hoursToDose(m, seed, r, dx, dz, 100.0));
                            for (double ticks : new double[]{0, 1000, 2400, 4000, 8000, 30000}) {
                                row("rain.rate", in(y, f, seed, hum, dx, dz, ticks),
                                        Fallout.doseRate(m, seed, r, dx, dz, ticks));
                            }
                        }
                        for (double sec : new double[]{0, 90, 100, 150, 200, 300, 400, 500, 600, 1000}) {
                            row("rain.strength", in(y, f, seed, hum, sec), r.strength(sec));
                        }
                    }
                }
            }
        }
    }

    // ---- FissionModel, ThermonuclearModel, Warhead ---------------------------------------------

    private void fission() {
        for (int u = -1; u <= 8; u++) {
            for (int p = -1; p <= 8; p++) {
                for (boolean t : new boolean[]{false, true}) {
                    FissionModel m = new FissionModel(u, p, t);
                    Warhead w = Warhead.of(m);
                    row("fission", in(u, p, t), m.uraniumCores, m.plutoniumCores, m.massKg, m.alpha, m.efficiency,
                            m.yieldKt, m.state.name(), m.nuclear(), w.yieldKt, w.fissionFraction, w.thermonuclear, w.fizzle);
                }
            }
        }
        for (double kt : new double[]{0.0, 0.0004, 0.0025, 0.0125, 0.052, 0.4995, 0.9995, 0.99999, 1.0, 1.25, 12.35,
                12.45, 99.94, 99.95, 99.96, 100.0, 115.5, 999.4, 999.5, 1000.0, 1250.0, 1255.0, 15110.0, 99999.0}) {
            row("format.yield", in(kt), FissionModel.format(kt));
        }
        for (double d : new double[]{0.0, 0.5, 1.5, 2.5, 640.4, 999.4, 999.5, 1000.0, 1050.0, 1950.0, 12345.0}) {
            row("format.distance", in(d), FissionModel.distance(d));
        }
        // The rounding of %.Nf itself (Java rounds the shortest decimal half up).
        double[] tricky = {0.0, 0.004, 0.005, 0.015, 0.025, 0.045, 0.125, 0.15, 0.35, 0.5, 0.6, 1.005, 1.255, 1.95,
                2.5, 2.675, 9.995, 12.35, 12.45, 99.95, 99.995, 1234.5, 0.0004, 1.0e-7, 7.0e-5, 123456.789, 5.55, 8.345,
                0.785, 1.0 / 3.0, 2.0 / 3.0, Math.PI, Math.E, 1.0e15 + 0.5, 4.35, 4.45, 0.0009765625};
        for (double v : tricky) {
            for (int p = 0; p <= 3; p++) {
                row("format.fixed", in(v, p), String.format(java.util.Locale.ROOT, "%." + p + "f", v));
            }
        }
        for (double kt : new double[]{0.0, 1.0, 150.0, 150.0001, 1000.0}) {
            for (double cap : new double[]{150.0, 1000.0}) {
                Warhead w = Warhead.ofYield(kt, cap);
                row("warhead.yield", in(kt, cap), w.yieldKt, w.fissionFraction, w.thermonuclear, w.fizzle, w.nuclear());
            }
        }
    }

    private void thermonuclear() {
        int[][] primaries = {{0, 0}, {1, 0}, {0, 2}, {2, 0}, {0, 3}, {3, 0}, {0, 6}, {6, 0}, {3, 3}, {1, 1}, {4, 0}, {0, 1}};
        for (int[] pr : primaries) {
            for (boolean tamper : new boolean[]{false, true}) {
                FissionModel p = new FissionModel(pr[0], pr[1], tamper);
                for (int c = -1; c <= 7; c++) {
                    for (boolean spark : new boolean[]{false, true}) {
                        for (boolean jacket : new boolean[]{false, true}) {
                            ThermonuclearModel m = new ThermonuclearModel(p, c, spark, jacket);
                            Warhead w = Warhead.of(m);
                            row("thermo", in(pr[0], pr[1], tamper, c, spark, jacket), m.charges, m.fuelKg, m.compression,
                                    m.burn, m.fusionKt, m.jacketKt, m.yieldKt, m.state.name(), m.nuclear(), m.fusion(),
                                    m.fissionFraction(), m.primaryNeededKt(), w.yieldKt, w.fissionFraction,
                                    w.thermonuclear, w.fizzle);
                        }
                    }
                }
            }
        }
    }

    // ---- Ballistics, SamGeometry, Intercept ----------------------------------------------------

    static final double[][] FLIGHTS = {{0.5, 64, 0.5, 0.5, 64, 0.5}, {0.5, 70, 0.5, 300.5, 64, -120.5},
            {-1000, 80, 2000, 1500, 63, -2500}, {12.5, 120, -7.5, 10012.5, 66, 3.5}, {0, 64, 0, 20000, 200, 20000},
            {100, 63, 100, 100, 63, 30100}, {2000, 70, 2000, -28000, 65, 2000}};

    private void ballistics() {
        for (double[] f : FLIGHTS) {
            Ballistics b = new Ballistics(f[0], f[1], f[2], f[3], f[4], f[5]);
            flight("ballistic", f, b);
        }
        double[][] shots = {{10.5, 65.95, 10.5, 400, 500, 300, 1.4}, {0, 64, 0, -1200, 900, 50, 3.1},
                {5, 70, 5, 5, 400, 5, 0.8}, {0, 64, 0, 0, 64, 0, 0.2}, {-30, 64, 40, 2500, 1500, -1800, 6.0}};
        for (double[] s : shots) {
            Ballistics b = Ballistics.interceptor(s[0], s[1], s[2], s[3], s[4], s[5], s[6]);
            flight("interceptor", s, b);
        }
        for (double r : new double[]{0, 1, 160, 1600, 20000}) {
            row("ballistics.seconds", in(r), Ballistics.flightSeconds(r));
        }
        fx();
    }

    /** The client's cloud and viewer curves (fx): what the volume shader and the overlay are fed. */
    private void fx() {
        double[] yields = {0.05, 1.0, 20.0, 115.0, 1000.0, 15000.0};
        double[] times = {-1.0, 0.0, 0.02, 0.3, 2.0, 10.0, 60.0, 300.0, 900.0, 2000.0};
        long[] seeds = {7L, -123456789L};
        SkyLight sky = new SkyLight();
        float[] p = new float[MushroomCloud.VOLUME_PARAMS];
        for (double y : yields) {
            BlastModel m = new BlastModel(y);
            for (long seed : seeds) {
                for (double scale : new double[]{0.75, 1.0}) {
                    MushroomCloud c = new MushroomCloud(m, seed, scale);
                    row("cloud.model", in(y, seed, scale), c.topHeight, c.capHeight, c.capRadius, c.capHalfHeight, c.stemRadius,
                            c.tauRise, c.tauForm, c.tauHeat, c.life, c.megaton);
                    for (double t : times) {
                        row("cloud.curves", in(y, seed, scale, t), c.heat(t), c.capCentre(t), c.fireballCentre(t), c.flash(t));
                        if (c.volume(t, 150.0, 40.0, -2500.0, sky, p)) {
                            double[] v = new double[p.length];
                            for (int i = 0; i < p.length; i++) v[i] = p[i];
                            row("cloud.volume", in(y, seed, scale, t), "some", v);
                        } else {
                            row("cloud.volume", in(y, seed, scale, t), "none");
                        }
                    }
                }
            }
            MushroomCloud c7 = new MushroomCloud(m, 7L, 0.75);
            for (double t : times) {
                for (double d : new double[]{10.0, 300.0, 1500.0, 4000.0, 20000.0}) {
                    for (boolean g : new boolean[]{true, false}) {
                        row("viewer.shake", in(y, t, d, g), ViewerEffects.shake(m, t, d * 0.9, d, g));
                    }
                    row("viewer.light", in(y, t, d), ViewerEffects.whiteout(c7, t, d, 0.6, 1.0), ViewerEffects.whiteout(c7, t, d, -0.4, 0.0),
                            ViewerEffects.glow(c7, t, d), ViewerEffects.tremor(m, d), ViewerEffects.tremorTime(m, d * 0.9, d));
                }
                float[] w = new float[4];
                WilsonCloud.params(m, t, 0.8, 1.0, w, 0);
                row("wilson", in(y, t), (double) w[0], (double) w[1], (double) w[2], (double) w[3]);
            }
        }
        for (double t : new double[]{0.0, 0.37, 5.0, 61.5}) {
            row("viewer.noise", in(t), ViewerEffects.noise(t, 1.3), ViewerEffects.noise(t, 7.7));
        }
        for (int day : new int[]{0, 1000, 6000, 11500, 12500, 13000, 18000, 23000}) {
            double angle = (float) (day / 24000.0 * Math.PI * 2.0);
            for (double rain : new double[]{0.0, 1.0}) {
                SkyLight k = new SkyLight();
                k.fromSun((float) angle, 0.7f, (float) rain);
                row("sky", in(angle, rain), (double) k.sunX, (double) k.sunY, (double) k.sunZ, (double) k.sunR,
                        (double) k.sunG, (double) k.sunB, (double) k.daylight, (double) k.ambR, (double) k.ambG, (double) k.ambB);
            }
        }
    }

    private void flight(String kind, double[] args, Ballistics b) {
        Object[] key = new Object[args.length];
        for (int i = 0; i < args.length; i++) key[i] = args[i];
        row(kind + ".path", key, b.range, b.apex, b.seconds, b.interceptor, b.burnSeconds(), b.ticks());
        List<Double> ts = new ArrayList<>();
        for (double t : new double[]{-1.0, 0.0, 0.01, 0.05, 0.3, 1.0, 2.5}) ts.add(t);
        for (int i = 1; i <= 24; i++) ts.add(b.seconds * i / 20.0);
        for (double t : ts) {
            Object[] k = new Object[args.length + 1];
            System.arraycopy(key, 0, k, 0, args.length);
            k[args.length] = t;
            row(kind + ".at", k, b.at(t), b.velocity(t), b.axis(t), b.nozzle(t), b.burning(t));
        }
    }

    private void sam() {
        for (int slot = -1; slot <= 4; slot++) {
            row("sam.rail", in(slot), SamGeometry.railX(slot), SamGeometry.railY(slot));
        }
        double[] yaws = {-4.0, -Math.PI, -1.0, 0.0, 0.3, Math.PI / 2, Math.PI, 3.5, 7.0, 100.0, -100.0};
        double[] els = {0.0, 0.17453292519943295, 0.8726646259971648, 1.2, -0.3};
        for (double yaw : yaws) {
            row("sam.wrap", in(yaw), SamGeometry.wrap(yaw));
            for (double el : els) {
                row("sam.forward", in(yaw, el), SamGeometry.forward(yaw, el));
                for (int slot = 0; slot < 4; slot++) {
                    row("sam.railCentre", in(10.0, 64.0, -20.0, yaw, el, slot),
                            SamGeometry.railCentre(10.0, 64.0, -20.0, yaw, el, slot));
                }
                row("sam.local", in(-3.0, 70.0, 5.0, yaw, el, 0.25, -0.5, 1.75),
                        SamGeometry.local(-3.0, 70.0, 5.0, yaw, el, 0.25, -0.5, 1.75));
            }
            for (double to : new double[]{-3.0, 0.0, 0.05, 1.0, 3.1}) {
                for (double dt : new double[]{-5.0, 0.0, 1.0, 3.0, 10.0, 40.0, 200.0}) {
                    row("sam.yaw", in(yaw, to, 100.0, 100.0 + dt), SamGeometry.yaw(yaw, to, 100.0, 100.0 + dt));
                }
            }
        }
        for (double[] p : new double[][]{{0, 0, 10, 10}, {0, 0, -10, 10}, {5, -5, 5.5, -5.5}, {5, -5, 5.5, 100}, {0, 0, -1000, -3}}) {
            row("sam.yawTowards", in(p[0], p[1], p[2], p[3]), SamGeometry.yawTowards(p[0], p[1], p[2], p[3]));
        }
        for (double from : new double[]{0.0, 0.3, 1.0}) {
            for (boolean up : new boolean[]{false, true}) {
                for (double dt : new double[]{-3.0, 0.0, 5.0, 20.0, 39.0, 40.0, 80.0}) {
                    row("sam.raise", in(from, up, 50.0, 50.0 + dt), SamGeometry.raise(from, up, 50.0, 50.0 + dt));
                }
            }
        }
        for (double r : new double[]{-0.5, 0.0, 0.1, 0.5, 0.77, 1.0, 1.5}) {
            row("sam.elevation", in(r), SamGeometry.elevationOf(r));
        }
    }

    private void intercept() {
        for (double[] f : FLIGHTS) {
            Ballistics b = new Ballistics(f[0], f[1], f[2], f[3], f[4], f[5]);
            double[][] batteries = {{f[3] + 200, 64, f[5] - 100}, {f[3], 63, f[5]}, {0.5 * (f[0] + f[3]), 70, 0.5 * (f[2] + f[5])},
                    {f[0] + 3000, 64, f[2]}, {f[0], 64, f[2] + 50}};
            for (double[] bat : batteries) {
                for (double age : new double[]{0.0, 2.0, 0.3 * b.seconds, 0.7 * b.seconds, b.seconds - 1.2}) {
                    for (double range : new double[]{1500.0, 4000.0}) {
                        for (double window : new double[]{0.5, 5.0, 100.0}) {
                            for (double after : new double[]{0.0, 0.5 * b.seconds}) {
                                Intercept i = Intercept.solve(b, age, bat[0], bat[1], bat[2], range, window, after);
                                Object[] key = in(f[0], f[1], f[2], f[3], f[4], f[5], age, bat[0], bat[1], bat[2], range, window, after);
                                if (i == null) row("intercept", key, "null");
                                else row("intercept", key, "some", i.meetAge, i.flight, i.x, i.y, i.z);
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- RadarHorizon, BurstDetection ----------------------------------------------------------

    private void radar() {
        double[][] pts = {{0, 1}, {1, 0}, {0, -1}, {-1, 0}, {1, 1}, {-1, -1}, {0, 0}, {-0.0, -1}, {1e-9, -1}, {-1e-9, -1},
                {3, 7}, {-250, 13}, {100, -0.1}};
        for (double[] p : pts) row("radar.bin", in(p[0], p[1]), RadarHorizon.bin(p[0], p[1]));
        for (int b = -1; b <= RadarHorizon.BINS; b += 7) {
            for (double k : new double[]{-0.5, 0.0, 0.25, 0.5}) row("radar.angle", in(b, k), RadarHorizon.angle(b, k));
        }
        for (double r : new double[]{0.0, 2.5, 10.0, 19.99, 20.0, 100.0, 256.0}) row("radar.next", in(r), RadarHorizon.next(r));
        for (double d : new double[]{-200, -90.5, -89.9, -0.5, -0.0, 0.0, 0.0001, 3.2, 89.1, 90.0, 120.0, Double.NaN}) {
            row("radar.encode", in(d), (int) RadarHorizon.encode(d));
        }
        byte[] horizon = new byte[RadarHorizon.BINS];
        for (int i = 0; i < horizon.length; i++) horizon[i] = (byte) ((i * 37) % 31 - 10);
        StringBuilder hb = new StringBuilder();
        for (byte b : horizon) hb.append(b).append(',');
        row("radar.horizon", in(), hb.toString());
        double[][] targets = {{100, 80, 0}, {0, 70, 100}, {-50, 64, -50}, {300, 140, -200}, {5, 400, 5}, {0, 66, 0},
                {-1000, 90, 300}, {20, 60, 20}};
        for (double[] t : targets) {
            row("radar.above", in(t[0], t[1], t[2]), RadarHorizon.above(horizon, 0.5, 66.35, 0.5, t[0], t[1], t[2]),
                    RadarHorizon.above(null, 0.5, 66.35, 0.5, t[0], t[1], t[2]));
        }
        row("radar.consts", in(), RadarHorizon.REACH, RadarHorizon.START, RadarHorizon.ANTENNA_Y);
    }

    private void burstDetection() {
        for (double y : new double[]{0.01, 1.0, 15.0, 115.0, 15000.0}) {
            for (double d : new double[]{0.0, 1000.0, 3000.0, 3000.5, 8000.0, 20000.0, 50000.0}) {
                for (long salt : SEEDS) {
                    BurstDetection b = BurstDetection.of(y, d, salt);
                    row("burst", in(y, d, salt), b.optical, b.estimateKt, b.error, b.delay);
                }
            }
        }
    }

    // ---- blast: RingColumns, LateGrid ----------------------------------------------------------

    /** FNV-1a over longs: the same in Rust, so a long list compares as one number. */
    static long fnv(long[] a, int n) {
        long h = 0xcbf29ce484222325L;
        for (int i = 0; i < n; i++) {
            h ^= a[i];
            h *= 0x100000001b3L;
        }
        return h;
    }

    private void rings() {
        for (int r : new int[]{0, 1, 2, 3, 4, 5, 7, 10, 16, 37, 100, 250, 1000}) {
            RingColumns rc = new RingColumns();
            rc.addRing(r);
            row("ring.all", in(r), rc.count, fnv(rc.cols, rc.count));
        }
        int[][] zeros = {{0, 0}, {7, -3}, {-1000, 2500}, {123456, -654321}};
        for (int[] g : zeros) {
            for (int r : new int[]{0, 3, 15, 16, 40, 100, 333}) {
                int cr = (r >> 4) + 2;
                for (int cx = (g[0] >> 4) - cr; cx <= (g[0] >> 4) + cr; cx++) {
                    for (int cz = (g[1] >> 4) - cr; cz <= (g[1] >> 4) + cr; cz++) {
                        boolean crosses = RingColumns.crosses(g[0], g[1], cx, cz, r);
                        RingColumns rc = new RingColumns();
                        rc.addChunk(g[0], g[1], cx, cz, r);
                        if (!crosses && rc.count == 0 && ((cx + cz) & 3) != 0) continue; // keep the table small
                        row("ring.chunk", in(g[0], g[1], cx, cz, r), crosses, rc.count, fnv(rc.cols, rc.count));
                    }
                }
            }
        }
        for (int[] v : new int[][]{{0, 0}, {1, -1}, {-70000, 33000}, {Integer.MAX_VALUE, Integer.MIN_VALUE}, {-5, 2147483}}) {
            long p = RingColumns.pack(v[0], v[1]);
            row("ring.pack", in(v[0], v[1]), p, RingColumns.dx(p), RingColumns.dz(p));
        }
    }

    private void lateGrid() {
        int[][] grids = {{0, 0, 0}, {100, -200, 37}, {-5000, 7000, 4000}, {3, 3, 900}};
        for (int[] g : grids) {
            LateGrid lg = LateGrid.forRadius(g[0], g[1], g[2]);
            String key = g[0] + "," + g[1] + "," + g[2];
            row("grid.new", in(g[0], g[1], g[2]), lg.cx0, lg.cz0, lg.r, lg.count(), lg.bytes());
            int side = 2 * lg.r + 1;
            // A fixed script of operations, logged step by step.
            StringBuilder log = new StringBuilder();
            long seed = 12345L + g[2];
            for (int step = 0; step < 400; step++) {
                seed = seed * 6364136223846793005L + 1442695040888963407L;
                int op = (int) ((seed >>> 33) % 5);
                int cx = lg.cx0 - lg.r - 2 + (int) ((seed >>> 13) % (side + 4));
                int cz = lg.cz0 - lg.r - 2 + (int) ((seed >>> 41) % (side + 4));
                int idx = lg.index(cx, cz);
                switch (op) {
                    case 0 -> log.append(lg.set(idx) ? 'S' : 's');
                    case 1 -> log.append(lg.clear(idx) ? 'C' : 'c');
                    case 2 -> log.append(lg.get(idx) ? 'G' : 'g');
                    case 3 -> { lg.setRow(cx, cz, cz + (int) ((seed >>> 7) % 70)); log.append('R'); }
                    default -> log.append(lg.next(idx)).append(';');
                }
                if (idx >= 0) log.append(lg.chunkX(idx) == cx && lg.chunkZ(idx) == cz ? '+' : '!');
            }
            long[] bits = lg.bits();
            row("grid.script", in(g[0], g[1], g[2]), log.toString(), lg.count(), fnv(bits, bits.length));
            LateGrid back = LateGrid.load(lg.cx0, lg.cz0, lg.r, bits);
            row("grid.load", in(g[0], g[1], g[2]), back.count(), back.next(0), back.next(side * side / 2));
        }
    }
}
