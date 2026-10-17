import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.eclipse.emf.ecore.*;

/**
 * Golden oracle for the SPD packages of crates/simoxide-model: the PcmModelOracle setup (generated
 * 5.2.2 packages) plus the generated SPD, semanticspd and spdmeasuringpoint packages of the
 * Slingshot jars (on the classpath, see run-spd.sh; their code links against the 5.2.2 PCM, EDP2
 * and Identifier classes). Loads every model file of each listed directory (PcmModelOracle's
 * extensions plus .spd and .semanticspd) and writes the same canonical dump.
 *
 * Usage: SpdOracle <plugins dir> <list file: name TAB dir per line> <out dir>
 */
public class SpdOracle {
    static final String[] PACKAGES = {
        "org.palladiosimulator.spd.SpdPackage",
        "org.palladiosimulator.semanticspd.SemanticspdPackage",
        "org.palladiosimulator.spdmeasuringpoint.SpdmeasuringpointPackage",
    };

    public static void main(String[] args) throws Exception {
        PcmModelOracle.setup(new File(args[0]));
        // touching eINSTANCE registers each package (and its subpackages) under its nsURI
        for (String c : PACKAGES) {
            EPackage p = (EPackage) Class.forName(c).getField("eINSTANCE").get(null);
            if (EPackage.Registry.INSTANCE.getEPackage(p.getNsURI()) != p) throw new IllegalStateException(c);
        }
        String[] ext = Arrays.copyOf(PcmModelOracle.EXT, PcmModelOracle.EXT.length + 2);
        ext[ext.length - 2] = "spd";
        ext[ext.length - 1] = "semanticspd";
        Path out = Paths.get(args[2]);
        Files.createDirectories(out);
        for (String line : Files.readAllLines(Paths.get(args[1]))) {
            if (line.isBlank() || line.startsWith("#")) continue;
            String[] kv = line.split("\t");
            try {
                String dump = PcmModelOracle.dumpDir(new File(kv[1]).getCanonicalFile(), ext);
                Files.write(out.resolve(kv[0] + ".jsonl"), dump.getBytes(StandardCharsets.UTF_8));
            } catch (Throwable t) {
                System.out.println("FAILED " + kv[0] + ": " + t);
            }
        }
    }
}
