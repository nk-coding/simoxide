package refsim;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.jar.JarFile;
import java.util.jar.Manifest;
import java.util.zip.ZipEntry;

import org.eclipse.core.runtime.ContributorFactorySimple;
import org.eclipse.core.runtime.IExtensionRegistry;
import org.eclipse.core.runtime.RegistryFactory;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.plugin.EcorePlugin;
import org.eclipse.emf.ecore.resource.URIConverter;

/**
 * Replaces OSGi for SimuLizar 5.2.2 on a flat classpath (derived from
 * palladio-research/work-simucom/standalone/StandaloneSimuLizar.java):
 * <ol>
 * <li>a standalone Eclipse extension registry filled from every product jar's plugin.xml, plus the
 * refsim recorder contribution;</li>
 * <li>platform:/plugin/&lt;bsn&gt;/ to jar:file:...!/ URI mappings;</li>
 * <li>EMF's standalone ExtensionProcessor (EPackages, factories, pathmaps).</li>
 * </ol>
 */
public final class Bootstrap {

    public static final String RECORDER_NAME = "refsim";

    private static final String REFSIM_PLUGIN_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
            + "<?eclipse version=\"3.4\"?>\n<plugin>\n"
            + "<extension id=\"refsim.recorder\" point=\"org.palladiosimulator.recorderframework.core\">\n"
            + "<recorder configurationFactory=\"refsim.RefsimRecorderConfigurationFactory\"\n"
            + "  recorderImplementation=\"refsim.RefsimRecorder\" name=\"" + RECORDER_NAME + "\"/>\n"
            + "</extension>\n</plugin>\n";

    private static boolean done;

    private Bootstrap() {
    }

    public static synchronized void init(File pluginsDir) throws Exception {
        if (done) {
            return;
        }
        // excluded: Architectural-Templates job/UI (need a workbench), simulizar.action (needs actionModelFile)
        String exclude = "^org\\.palladiosimulator\\.(architecturaltemplates\\.(jobs|ui)|simulizar\\.action.*)$";
        Object masterToken = new Object();
        IExtensionRegistry registry = RegistryFactory.createRegistry(null, masterToken, null);
        File[] jars = pluginsDir.listFiles((d, n) -> n.endsWith(".jar"));
        Arrays.sort(jars); // deterministic contribution order
        for (File jar : jars) {
            try (JarFile jf = new JarFile(jar)) {
                Manifest mf = jf.getManifest();
                if (mf == null) {
                    continue;
                }
                String bsn = mf.getMainAttributes().getValue("Bundle-SymbolicName");
                if (bsn == null) {
                    continue;
                }
                bsn = bsn.split(";")[0].trim();
                URIConverter.URI_MAP.put(URI.createURI("platform:/plugin/" + bsn + "/"),
                        URI.createURI("jar:" + jar.toURI() + "!/"));
                if (bsn.matches(exclude)) {
                    continue;
                }
                ZipEntry pe = jf.getEntry("plugin.xml");
                if (pe == null) {
                    continue;
                }
                try (InputStream in = jf.getInputStream(pe)) {
                    registry.addContribution(in, ContributorFactorySimple.createContributor(bsn), false, bsn, null,
                            masterToken);
                }
            }
        }
        registry.addContribution(new ByteArrayInputStream(REFSIM_PLUGIN_XML.getBytes(StandardCharsets.UTF_8)),
                ContributorFactorySimple.createContributor("refsim"), false, "refsim", null, masterToken);
        RegistryFactory.setDefaultRegistryProvider(() -> registry);

        ClassLoader noPluginXml = new ClassLoader(Bootstrap.class.getClassLoader()) {
            @Override
            public java.util.Enumeration<java.net.URL> getResources(String name) throws java.io.IOException {
                return "plugin.xml".equals(name) ? java.util.Collections.emptyEnumeration() : super.getResources(name);
            }
        };
        EcorePlugin.ExtensionProcessor.process(noPluginXml);
        org.apache.log4j.BasicConfigurator.configure();
        org.apache.log4j.Logger.getRootLogger()
            .setLevel(org.apache.log4j.Level.toLevel(System.getProperty("refsim.log", "ERROR")));
        // PATCH (state, not class): SimuLizar's synthetic system assembly context is a static EObject whose id
        // is a random UUID generated once per JVM (Identifier default). It shows up in the interpreter's
        // assembly context stack; give it a fixed id so fresh JVMs behave (and trace) identically.
        if (!Statics.UNPATCHED) {
            org.palladiosimulator.simulizar.interpreter.RepositoryComponentSwitch.SYSTEM_ASSEMBLY_CONTEXT
                .setId(SYSTEM_ASSEMBLY_CONTEXT_ID);
        }
        done = true;
    }

    public static final String SYSTEM_ASSEMBLY_CONTEXT_ID = "_SYSTEM_ASSEMBLY_CONTEXT_";
}
