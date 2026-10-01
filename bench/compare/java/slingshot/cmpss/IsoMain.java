package cmpss;

import java.io.File;
import java.io.OutputStream;
import java.io.PrintStream;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * K Slingshot instances in one JVM, each in its own class loader (Slingshot's global singletons and
 * the StoEx serialiser are not thread-safe; isolation is the working way to run it in parallel).
 * Usage: java -cp &lt;this dir only&gt; cmpss.IsoMain K classpath.txt &lt;CompareSlingshot args&gt;
 * Each instance gets tag=iso&lt;k&gt; (for the file barrier).
 */
public final class IsoMain {

    public static void main(String[] args) throws Exception {
        int k = Integer.parseInt(args[0]);
        List<URL> urls = new ArrayList<>();
        for (String line : Files.readAllLines(new File(args[1]).toPath())) {
            for (String e : line.split(":")) {
                if (!e.isBlank()) {
                    urls.add(new File(e.trim()).toURI().toURL());
                }
            }
        }
        String[] rest = Arrays.copyOfRange(args, 2, args.length);
        System.getProperties().put("cmp.realOut", System.out);
        System.getProperties().put("cmp.realErr", System.err);
        if (!Boolean.getBoolean("cmp.verbose")) {
            PrintStream sink = new PrintStream(OutputStream.nullOutputStream());
            System.setOut(sink);
            System.setErr(sink);
        }
        List<Thread> ts = new ArrayList<>();
        for (int i = 0; i < k; i++) {
            final int inst = i;
            URLClassLoader cl = new URLClassLoader("iso" + i, urls.toArray(new URL[0]),
                    ClassLoader.getPlatformClassLoader());
            String[] a = Arrays.copyOf(rest, rest.length + 1);
            a[rest.length] = "tag=iso" + inst;
            Thread t = new Thread(() -> {
                try {
                    Class<?> c = Class.forName("cmpss.CompareSlingshot", true, cl);
                    Method m = c.getMethod("instance", String[].class);
                    m.invoke(null, (Object) a);
                } catch (Throwable e) {
                    e.printStackTrace((PrintStream) System.getProperties().get("cmp.realErr"));
                }
            }, "iso" + i);
            t.setContextClassLoader(cl);
            ts.add(t);
            t.start();
        }
        for (Thread t : ts) {
            t.join();
        }
        System.exit(0);
    }
}
