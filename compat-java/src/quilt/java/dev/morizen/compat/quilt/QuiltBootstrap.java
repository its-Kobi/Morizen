package dev.morizen.compat.quilt;

import dev.morizen.compat.NativeBridge;
import dev.morizen.compat.events.BridgeEvents;
import java.nio.file.Path;
import java.util.Comparator;
import org.quiltmc.loader.api.ModContainer;
import org.quiltmc.loader.api.QuiltLoader;
import org.quiltmc.loader.impl.game.minecraft.Hooks;

/** Runs Quilt Loader entrypoints inside Knot's target class loader. */
public final class QuiltBootstrap {
    private static ClassLoader targetLoader;

    private QuiltBootstrap() {}

    public static void start(Path gameDirectory) {
        ClassLoader ownLoader = QuiltBootstrap.class.getClassLoader();
        if (ownLoader == ClassLoader.getSystemClassLoader()) {
            throw new IllegalStateException("Morizen Quilt bootstrap was loaded outside Knot");
        }
        targetLoader = ownLoader;
        Thread thread = Thread.currentThread();
        thread.setName("Java mods");
        thread.setContextClassLoader(ownLoader);

        String loaderVersion = QuiltLoader.getModContainer("quilt_loader")
                .orElseThrow(() -> new IllegalStateException("Quilt Loader has no own mod container"))
                .metadata()
                .version()
                .raw();
        System.out.println("morizen-java: quilt-loader=" + loaderVersion);
        QuiltLoader.getAllMods().stream()
                .sorted(Comparator.comparing(container -> container.metadata().id()))
                .map(QuiltBootstrap::describe)
                .forEach(description -> System.out.println("morizen-java: quilt-mod=" + description));

        Throwable failure = null;
        try {
            Hooks.startClient(gameDirectory.toFile(), new MorizenQuiltGameInstance());
            for (QuiltInitializer initializer :
                    QuiltLoader.getEntrypoints("morizen", QuiltInitializer.class)) {
                initializer.onInitialize();
            }
        } catch (Throwable error) {
            failure = error;
            throw error;
        } finally {
            try {
                NativeBridge.commitRegistry();
            } catch (Throwable commitError) {
                if (failure != null) {
                    failure.addSuppressed(commitError);
                } else {
                    throw commitError;
                }
            }
        }
    }

    private static String describe(ModContainer container) {
        return container.metadata().id() + "@" + container.metadata().version().raw();
    }

    public static String dispatch(String wire) {
        return BridgeEvents.dispatch(wire);
    }

    public static boolean hasCallbacks() {
        return BridgeEvents.hasCallbacks();
    }

    public static void stop() {
        System.out.println("morizen-java: quilt bootstrap stop");
        targetLoader = null;
        Thread.currentThread().setContextClassLoader(ClassLoader.getSystemClassLoader());
    }

    private static final class MorizenQuiltGameInstance {}
}
